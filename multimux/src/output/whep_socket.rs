//! The media sockets of a WHEP route: one UDP socket per advertised IP, shared
//! by every viewer, instead of one socket per viewer.
//!
//! Which viewer a datagram belongs to is `webrtc_runtime::media::SocketDemux`'s
//! decision (ICE ufrag, then mapped source address); this module owns only the
//! IO around it: the receive loop, a bounded queue per viewer, and the socket
//! the viewers send through.
//!
//! A viewer that cannot keep up loses its newest datagrams (`try_send` into a
//! full queue) instead of making the receive loop, and so every other viewer,
//! wait.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use parking_lot::Mutex;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use webrtc_runtime::media::{MAX_REMOTE_CANDIDATES, SocketDemux};

/// Largest datagram read: one Ethernet MTU. Inbound WHEP traffic is STUN,
/// DTLS and RTCP, all of which fit.
const RECV_BUFFER_BYTES: usize = 1500;

/// Datagrams one viewer may have queued before further ones are dropped.
const SESSION_INBOUND_QUEUE: usize = 64;

/// Wait before retrying `recv_from` after a failed receive. A persistent
/// error (a closed descriptor, `ENOBUFS`) would otherwise busy-loop the task
/// and starve every viewer on the same socket.
const RECV_ERROR_BACKOFF: Duration = Duration::from_millis(10);

/// Consecutive receive errors after which the loop gives the socket up and
/// marks it dead, so the next `socket_for` binds a replacement rather than
/// handing out a socket nothing reads.
const RECV_ERRORS_BEFORE_DEAD: u32 = 10;

type SessionKey = u64;

/// One inbound datagram and where it came from.
pub(crate) struct Inbound {
    pub peer: SocketAddr,
    pub bytes: Bytes,
}

struct Router {
    demux: SocketDemux<SessionKey>,
    queues: HashMap<SessionKey, mpsc::Sender<Inbound>>,
}

/// One bound socket and the receive loop feeding its viewers.
pub(crate) struct SharedSocket {
    socket: Arc<UdpSocket>,
    local_addr: SocketAddr,
    router: Arc<Mutex<Router>>,
    next_key: AtomicU64,
    /// Set when the receive loop ends; the next `socket_for` rebinds.
    dead: Arc<AtomicBool>,
    #[cfg(test)]
    dropped: Arc<AtomicU64>,
    #[cfg(test)]
    handled: Arc<AtomicU64>,
    task: JoinHandle<()>,
}

impl Drop for SharedSocket {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl SharedSocket {
    fn bind(ip: IpAddr) -> io::Result<Self> {
        let std_socket = std::net::UdpSocket::bind((ip, 0))?;
        std_socket.set_nonblocking(true)?;
        let socket = Arc::new(UdpSocket::from_std(std_socket)?);
        let local_addr = socket.local_addr()?;
        let router = Arc::new(Mutex::new(Router {
            demux: SocketDemux::new(MAX_REMOTE_CANDIDATES),
            queues: HashMap::new(),
        }));
        let dropped = Arc::new(AtomicU64::new(0));
        let handled = Arc::new(AtomicU64::new(0));
        let dead = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(receive_loop(
            Arc::clone(&socket),
            Arc::clone(&router),
            Arc::clone(&dropped),
            Arc::clone(&handled),
            Arc::clone(&dead),
        ));
        Ok(Self {
            socket,
            local_addr,
            router,
            next_key: AtomicU64::new(0),
            dead,
            #[cfg(test)]
            dropped,
            #[cfg(test)]
            handled,
            task,
        })
    }

    /// The address every viewer on this socket advertises as its candidate.
    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The socket viewers send through.
    pub(crate) fn socket(&self) -> Arc<UdpSocket> {
        Arc::clone(&self.socket)
    }

    /// Datagrams dropped because a viewer's queue was full.
    #[cfg(test)]
    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Datagrams the receive loop has finished with: classified, and queued or dropped.
    #[cfg(test)]
    fn handled(&self) -> u64 {
        self.handled.load(Ordering::Acquire)
    }

    /// Route datagrams naming `local_ufrag` to a new viewer, authenticating
    /// its connectivity checks with `local_pwd`. `None` when another viewer
    /// already owns that ufrag.
    pub(crate) fn attach(
        self: &Arc<Self>,
        local_ufrag: &str,
        local_pwd: &str,
    ) -> Option<(Attachment, mpsc::Receiver<Inbound>)> {
        let key = self.next_key.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(SESSION_INBOUND_QUEUE);
        {
            let mut router = self.router.lock();
            if !router.demux.register(local_ufrag, local_pwd, key) {
                return None;
            }
            router.queues.insert(key, tx);
        }
        Some((
            Attachment {
                shared: Arc::clone(self),
                key,
            },
            rx,
        ))
    }
}

/// A datagram source: the real [`UdpSocket`], or a test's injected
/// (always-failing) receiver.
trait RecvFrom {
    async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)>;
}

impl RecvFrom for Arc<UdpSocket> {
    async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        (**self).recv_from(buf).await
    }
}

async fn receive_loop<S: RecvFrom>(
    source: S,
    router: Arc<Mutex<Router>>,
    dropped: Arc<AtomicU64>,
    handled: Arc<AtomicU64>,
    dead: Arc<AtomicBool>,
) {
    let mut buf = [0u8; RECV_BUFFER_BYTES];
    let mut consecutive_errors = 0u32;
    loop {
        let (len, peer) = match source.recv_from(&mut buf).await {
            Ok(ok) => {
                consecutive_errors = 0;
                ok
            }
            Err(e) => {
                consecutive_errors = consecutive_errors.saturating_add(1);
                if consecutive_errors == 1 {
                    tracing::warn!(error = %e, "whep: media socket receive failed; backing off");
                }
                if consecutive_errors >= RECV_ERRORS_BEFORE_DEAD {
                    tracing::warn!(
                        error = %e,
                        errors = consecutive_errors,
                        "whep: media socket receive keeps failing; closing it"
                    );
                    break;
                }
                tokio::time::sleep(RECV_ERROR_BACKOFF).await;
                continue;
            }
        };
        let data = &buf[..len];
        let queue = {
            let mut router = router.lock();
            router
                .demux
                .classify(data, peer)
                .and_then(|key| router.queues.get(&key).cloned())
        };
        if let Some(queue) = queue {
            let datagram = Inbound {
                peer,
                bytes: Bytes::copy_from_slice(data),
            };
            if let Err(mpsc::error::TrySendError::Full(_)) = queue.try_send(datagram) {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        handled.fetch_add(1, Ordering::Release);
    }
    dead.store(true, Ordering::SeqCst);
}

/// A viewer's registration on a [`SharedSocket`]; dropping it unregisters the
/// viewer and everything it mapped.
pub(crate) struct Attachment {
    shared: Arc<SharedSocket>,
    key: SessionKey,
}

impl Attachment {
    /// Also route datagrams naming `local_ufrag` (the one an ICE restart
    /// introduces) to this viewer, authenticating its checks with `local_pwd`.
    pub(crate) fn register_ufrag(&self, local_ufrag: &str, local_pwd: &str) {
        if !self
            .shared
            .router
            .lock()
            .demux
            .register(local_ufrag, local_pwd, self.key)
        {
            tracing::warn!("whep: restart ufrag already belongs to another viewer");
        }
    }
}

impl Drop for Attachment {
    fn drop(&mut self) {
        let mut router = self.shared.router.lock();
        router.demux.unregister(&self.key);
        router.queues.remove(&self.key);
    }
}

/// The media sockets of one advertised IP: bound one by one as viewers arrive up to
/// the configured count, then handed out round-robin.
#[derive(Default)]
struct IpSockets {
    shards: Vec<Arc<SharedSocket>>,
    next: usize,
}

/// A route's media sockets, up to `per_ip` per advertised IP, bound on first use.
pub(crate) struct SharedSockets {
    per_ip: usize,
    sockets: Mutex<HashMap<IpAddr, IpSockets>>,
}

impl Default for SharedSockets {
    fn default() -> Self {
        Self::with_sockets_per_ip(crate::config::DEFAULT_WHEP_MEDIA_SOCKETS_PER_IP)
    }
}

impl SharedSockets {
    /// Sockets per advertised IP: clamped up to one.
    pub(crate) fn with_sockets_per_ip(per_ip: usize) -> Self {
        Self {
            per_ip: per_ip.max(1),
            sockets: Mutex::new(HashMap::new()),
        }
    }

    /// A socket for `ip`: a newly bound one while the IP has fewer than the configured count,
    /// otherwise the next in rotation. A rotation slot whose socket died (its
    /// receive loop ended) is rebound here rather than handed out.
    pub(crate) fn socket_for(&self, ip: IpAddr) -> io::Result<Arc<SharedSocket>> {
        let mut sockets = self.sockets.lock();
        let entry = sockets.entry(ip).or_default();
        if entry.shards.len() < self.per_ip {
            let socket = Arc::new(SharedSocket::bind(ip)?);
            entry.shards.push(Arc::clone(&socket));
            return Ok(socket);
        }
        let index = entry.next % entry.shards.len();
        entry.next = entry.next.wrapping_add(1);
        if entry.shards[index].dead.load(Ordering::SeqCst) {
            let socket = Arc::new(SharedSocket::bind(ip)?);
            entry.shards[index] = Arc::clone(&socket);
            return Ok(socket);
        }
        Ok(Arc::clone(&entry.shards[index]))
    }

    /// How many sockets are bound.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.sockets.lock().values().map(|ip| ip.shards.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtc_stun::attributes::ATTR_USERNAME;
    use rtc_stun::fingerprint::FINGERPRINT;
    use rtc_stun::integrity::MessageIntegrity;
    use rtc_stun::message::{BINDING_REQUEST, Message, TransactionId};
    use rtc_stun::textattrs::TextAttribute;
    use std::net::Ipv4Addr;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    const DTLS_RECORD: [u8; 4] = [22, 254, 253, 0];
    const HANDLED_DEADLINE: Duration = Duration::from_secs(60);
    /// The ice-pwd every viewer in these tests is attached with, so a check
    /// naming any of them can carry a verifiable `MESSAGE-INTEGRITY`.
    const PWD: &str = "whep-socket-ice-password-0";

    fn check(username: &str) -> Vec<u8> {
        let crypto = rtc_stun::crypto::default_provider().expect("default crypto provider");
        let mut message = Message::new();
        message
            .build(&[
                Box::new(BINDING_REQUEST),
                Box::new(TransactionId::new()),
                Box::new(TextAttribute::new(ATTR_USERNAME, username.to_string())),
                Box::new(MessageIntegrity::new_short_term_integrity_with_provider(
                    PWD.to_string(),
                    crypto.crypto(),
                )),
                Box::new(FINGERPRINT),
            ])
            .expect("build a binding request");
        message.raw
    }

    /// A check with no `MESSAGE-INTEGRITY`: any sender who knows a ufrag can
    /// produce one, but only the peer knows the password.
    fn bare_check(username: &str) -> Vec<u8> {
        let mut message = Message::new();
        message
            .build(&[
                Box::new(BINDING_REQUEST),
                Box::new(TransactionId::new()),
                Box::new(TextAttribute::new(ATTR_USERNAME, username.to_string())),
                Box::new(FINGERPRINT),
            ])
            .expect("build a bare binding request");
        message.raw
    }

    /// Waits until the receive loop has finished with `count` datagrams. Every assertion below
    /// reads a viewer's queue only after this, so a "nothing else arrived" check covers every
    /// datagram that was sent, however the scheduler orders the senders and the loop.
    async fn wait_handled(shared: &SharedSocket, count: u64) {
        let deadline = tokio::time::Instant::now() + HANDLED_DEADLINE;
        while shared.handled() < count {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the receive loop handled {} of {count} datagrams",
                shared.handled()
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    async fn client() -> UdpSocket {
        UdpSocket::bind((LOCALHOST, 0)).await.expect("bind client")
    }

    /// The viewers of a route on one IP spread evenly over exactly the configured number of
    /// sockets, whatever that number is.
    #[tokio::test]
    async fn viewers_of_a_route_spread_over_the_configured_number_of_sockets() {
        const ROUNDS: usize = 3;
        for per_ip in [1, 3, 8] {
            let sockets = SharedSockets::with_sockets_per_ip(per_ip);
            let mut attached = Vec::new();
            let mut per_address: HashMap<SocketAddr, usize> = HashMap::new();
            for i in 0..per_ip * ROUNDS {
                let socket = sockets.socket_for(LOCALHOST).expect("socket");
                *per_address.entry(socket.local_addr()).or_default() += 1;
                attached.push(socket.attach(&format!("ufrag{i:04}"), PWD).expect("attach"));
            }
            assert_eq!(sockets.len(), per_ip, "bound sockets");
            assert_eq!(per_address.len(), per_ip, "distinct addresses");
            assert!(per_address.values().all(|viewers| *viewers == ROUNDS));
        }
    }

    #[test]
    fn a_zero_count_is_clamped_to_one_socket() {
        assert_eq!(SharedSockets::with_sockets_per_ip(0).per_ip, 1);
    }

    /// A check reaches the viewer it names, later datagrams from that address
    /// follow it, and another viewer sees none of it.
    #[tokio::test]
    async fn datagrams_reach_only_the_viewer_that_owns_the_sender() {
        let sockets = SharedSockets::default();
        let shared = sockets.socket_for(LOCALHOST).expect("socket");
        let (_a, mut a_rx) = shared.attach("aaaaaaaa", PWD).expect("attach a");
        let (_b, mut b_rx) = shared.attach("bbbbbbbb", PWD).expect("attach b");
        let peer = client().await;
        peer.send_to(&check("bbbbbbbb:peer"), shared.local_addr())
            .await
            .expect("send check");
        peer.send_to(&DTLS_RECORD, shared.local_addr())
            .await
            .expect("send dtls");
        wait_handled(&shared, 2).await;
        for _ in 0..2 {
            let got = b_rx.try_recv().expect("b has the datagram");
            assert_eq!(got.peer, peer.local_addr().expect("addr"));
        }
        assert!(b_rx.try_recv().is_err(), "b has nothing else");
        assert!(a_rx.try_recv().is_err());
    }

    /// A check naming an unknown ufrag, and datagrams from an unmapped
    /// address, reach nobody.
    #[tokio::test]
    async fn a_spoofed_check_reaches_nobody() {
        let sockets = SharedSockets::default();
        let shared = sockets.socket_for(LOCALHOST).expect("socket");
        let (_a, mut a_rx) = shared.attach("aaaaaaaa", PWD).expect("attach");
        let peer = client().await;
        peer.send_to(&check("zzzzzzzz:peer"), shared.local_addr())
            .await
            .expect("send");
        peer.send_to(&DTLS_RECORD, shared.local_addr())
            .await
            .expect("send");
        let mapped = client().await;
        mapped
            .send_to(&check("aaaaaaaa:peer"), shared.local_addr())
            .await
            .expect("send");
        wait_handled(&shared, 3).await;
        let got = a_rx.try_recv().expect("a has the real check");
        assert_eq!(got.peer, mapped.local_addr().expect("addr"));
        assert!(a_rx.try_recv().is_err(), "only the real check arrived");
    }

    /// A viewer whose queue is full loses datagrams; the receive loop goes on
    /// serving the others.
    #[tokio::test]
    async fn a_full_viewer_queue_drops_without_blocking_the_others() {
        let sockets = SharedSockets::default();
        let shared = sockets.socket_for(LOCALHOST).expect("socket");
        let (_slow, _slow_rx) = shared.attach("aaaaaaaa", PWD).expect("attach slow");
        let (_fast, mut fast_rx) = shared.attach("bbbbbbbb", PWD).expect("attach fast");
        let slow_peer = client().await;
        let fast_peer = client().await;
        slow_peer
            .send_to(&check("aaaaaaaa:p"), shared.local_addr())
            .await
            .expect("send");
        for _ in 0..SESSION_INBOUND_QUEUE * 2 {
            slow_peer
                .send_to(&DTLS_RECORD, shared.local_addr())
                .await
                .expect("send");
        }
        fast_peer
            .send_to(&check("bbbbbbbb:p"), shared.local_addr())
            .await
            .expect("send");
        wait_handled(&shared, (SESSION_INBOUND_QUEUE * 2 + 2) as u64).await;
        let got = fast_rx
            .try_recv()
            .expect("the other viewer is still served");
        assert_eq!(got.peer, fast_peer.local_addr().expect("addr"));
        assert!(shared.dropped() > 0, "the full queue dropped datagrams");
    }

    /// Dropping an attachment frees its ufrag.
    #[tokio::test]
    async fn a_dropped_attachment_frees_its_ufrag() {
        let sockets = SharedSockets::default();
        let shared = sockets.socket_for(LOCALHOST).expect("socket");
        let first = shared.attach("aaaaaaaa", PWD).expect("attach");
        assert!(shared.attach("aaaaaaaa", PWD).is_none());
        drop(first);
        assert!(shared.attach("aaaaaaaa", PWD).is_some());
    }

    /// Audit 20 finding 40: a check naming a viewer's real ufrag reaches it
    /// only when its `MESSAGE-INTEGRITY` verifies under that viewer's
    /// password, so a ufrag-knowing sender cannot map an address to it.
    #[tokio::test]
    async fn a_check_without_the_password_reaches_nobody() {
        let sockets = SharedSockets::default();
        let shared = sockets.socket_for(LOCALHOST).expect("socket");
        let (_a, mut a_rx) = shared.attach("aaaaaaaa", PWD).expect("attach a");
        let spoof = client().await;
        spoof
            .send_to(&bare_check("aaaaaaaa:peer"), shared.local_addr())
            .await
            .expect("send bare check");
        let peer = client().await;
        peer.send_to(&check("aaaaaaaa:peer"), shared.local_addr())
            .await
            .expect("send authenticated check");
        wait_handled(&shared, 2).await;
        let got = a_rx.try_recv().expect("a has the authenticated check");
        assert_eq!(got.peer, peer.local_addr().expect("addr"));
        assert!(
            a_rx.try_recv().is_err(),
            "only the authenticated check arrived"
        );
    }

    /// A receive error that never clears backs off rather than spinning, and
    /// after the consecutive-error limit the loop ends and marks the socket
    /// dead (audit 20 finding 23 — the old loop `continue`d with no bound).
    /// MUTATION VERIFIED: restoring a bare `continue` makes this test's outer
    /// timeout expire instead of the loop ending.
    #[tokio::test]
    async fn a_persistent_receive_error_backs_off_then_ends_the_loop() {
        struct FailingSource;
        impl RecvFrom for FailingSource {
            async fn recv_from(&self, _buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
                Err(io::Error::from_raw_os_error(105 /* ENOBUFS */))
            }
        }

        let router = Arc::new(Mutex::new(Router {
            demux: SocketDemux::new(MAX_REMOTE_CANDIDATES),
            queues: HashMap::new(),
        }));
        let dead = Arc::new(AtomicBool::new(false));
        let started = tokio::time::Instant::now();
        tokio::time::timeout(
            Duration::from_secs(5),
            receive_loop(
                FailingSource,
                router,
                Arc::new(AtomicU64::new(0)),
                Arc::new(AtomicU64::new(0)),
                Arc::clone(&dead),
            ),
        )
        .await
        .expect("the loop ends after the consecutive-error limit");
        assert!(
            dead.load(Ordering::SeqCst),
            "an ended loop marks the socket dead"
        );
        assert!(
            started.elapsed() >= RECV_ERROR_BACKOFF * (RECV_ERRORS_BEFORE_DEAD - 1),
            "it backed off between retries rather than spinning: {:?}",
            started.elapsed()
        );
    }

    /// A socket whose receive loop ended is rebound by `socket_for`, so a new
    /// viewer never attaches to a socket nothing reads.
    #[tokio::test]
    async fn a_dead_socket_is_rebound_on_the_next_socket_for() {
        let sockets = SharedSockets::with_sockets_per_ip(1);
        let first = sockets.socket_for(LOCALHOST).expect("socket");
        first.dead.store(true, Ordering::SeqCst);
        let second = sockets.socket_for(LOCALHOST).expect("rebound");
        assert!(!Arc::ptr_eq(&first, &second), "the dead socket is replaced");
        assert_ne!(first.local_addr(), second.local_addr());
        assert_eq!(sockets.len(), 1, "the dead socket is replaced in place");
    }

    /// A success between errors resets the consecutive-error count: ten
    /// alternating failures and datagrams must not end the loop, or one
    /// sporadic error every retry would retire a healthy socket.
    /// MUTATION VERIFIED: removing `consecutive_errors = 0` from the `Ok` arm
    /// ends the loop after the tenth error and marks the socket dead.
    #[tokio::test]
    async fn a_successful_receive_resets_the_consecutive_error_count() {
        struct AlternatingSource {
            calls: AtomicUsize,
        }
        impl RecvFrom for AlternatingSource {
            async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
                if self.calls.fetch_add(1, Ordering::SeqCst).is_multiple_of(2) {
                    Err(io::Error::from_raw_os_error(105 /* ENOBUFS */))
                } else {
                    buf[0] = 1;
                    Ok((1, SocketAddr::from(([127, 0, 0, 1], 9))))
                }
            }
        }

        let router = Arc::new(Mutex::new(Router {
            demux: SocketDemux::new(MAX_REMOTE_CANDIDATES),
            queues: HashMap::new(),
        }));
        let handled = Arc::new(AtomicU64::new(0));
        let dead = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(receive_loop(
            AlternatingSource {
                calls: AtomicUsize::new(0),
            },
            router,
            Arc::new(AtomicU64::new(0)),
            Arc::clone(&handled),
            Arc::clone(&dead),
        ));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while handled.load(Ordering::Acquire) < RECV_ERRORS_BEFORE_DEAD as u64 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the loop handled {} of {RECV_ERRORS_BEFORE_DEAD} datagrams",
                handled.load(Ordering::Acquire)
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(
            !dead.load(Ordering::SeqCst),
            "a success between errors must keep the socket alive"
        );
        task.abort();
    }
}
