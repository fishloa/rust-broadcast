//! Shared codec for the fixed-size entry-list descriptors of the INT / UNT
//! target loops (ETSI EN 301 192 §8.4.5 Tables 24-31, ETSI TS 102 006
//! §9.5.2 Tables 21-23) and the RNT scan descriptor (ETSI TS 102 323 Table 14).
//!
//! Every one of those descriptors is `descriptor_tag`, `descriptor_length`,
//! an optional fixed-size leading field (an address mask), then a repeat of a
//! fixed-size entry up to `descriptor_length`. The spec defines nothing after
//! the last whole entry. `trailing` is therefore **parse tolerance only**: a
//! non-conformant sender's partial tail (strictly shorter than one entry) is
//! kept verbatim so it re-serializes byte-identically. Serialize rejects a
//! hand-built `trailing` of one entry or more (`check_trailing`), because it
//! would re-parse as extra entries and break `parse(serialize(x)) == x`.

use super::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use core::net::{Ipv4Addr, Ipv6Addr};

/// Wire width of a descriptor's `descriptor_tag` + `descriptor_length`.
pub(crate) const HEADER_LEN: usize = 2;

/// A 48-bit MAC address as 6 octets, most significant octet first.
pub type MacAddress = [u8; 6];

/// A fixed-size wire value (an address) of `N` octets.
pub(crate) trait Octets<const N: usize>: Copy {
    fn from_octets(b: [u8; N]) -> Self;
    fn to_octets(self) -> [u8; N];
}

impl Octets<4> for Ipv4Addr {
    fn from_octets(b: [u8; 4]) -> Self {
        Self::from(b)
    }
    fn to_octets(self) -> [u8; 4] {
        self.octets()
    }
}

impl Octets<16> for Ipv6Addr {
    fn from_octets(b: [u8; 16]) -> Self {
        Self::from(b)
    }
    fn to_octets(self) -> [u8; 16] {
        self.octets()
    }
}

impl Octets<6> for [u8; 6] {
    fn from_octets(b: [u8; 6]) -> Self {
        b
    }
    fn to_octets(self) -> [u8; 6] {
        self
    }
}

/// A fixed-size descriptor-body entry of `E` octets.
pub(crate) trait Entry<const E: usize>: Sized {
    fn read(b: &[u8; E]) -> Self;
    fn write(&self, b: &mut [u8; E]);
}

/// Reject a `trailing` that is not a strictly partial entry (`>= entry_len`
/// bytes would re-parse as whole entries).
pub(crate) fn check_trailing(tag: u8, trailing: &[u8], entry_len: usize) -> Result<()> {
    if trailing.len() >= entry_len {
        return Err(Error::InvalidDescriptor {
            tag,
            reason: "trailing is not shorter than one entry (would re-parse as entries)",
        });
    }
    Ok(())
}

/// Split `body` into whole `E`-octet entries and the partial tail.
pub(crate) fn parse_entries<const E: usize, T: Entry<E>>(body: &[u8]) -> (Vec<T>, &[u8]) {
    let (chunks, tail) = body.as_chunks::<E>();
    (chunks.iter().map(T::read).collect(), tail)
}

/// Write `entries` then `trailing` at `buf[off..]`; returns the end offset.
pub(crate) fn write_entries<const E: usize, T: Entry<E>>(
    buf: &mut [u8],
    mut off: usize,
    entries: &[T],
    trailing: &[u8],
) -> usize {
    for e in entries {
        let mut b = [0u8; E];
        e.write(&mut b);
        buf[off..off + E].copy_from_slice(&b);
        off += E;
    }
    buf[off..off + trailing.len()].copy_from_slice(trailing);
    off + trailing.len()
}

/// Parse `[mask][entry]*[tail]` where `mask` is one `N`-octet address.
pub(crate) fn parse_masked<'a, const N: usize, A: Octets<N>>(
    bytes: &'a [u8],
    tag: u8,
    what: &'static str,
) -> Result<(A, Vec<A>, &'a [u8])> {
    let body = descriptor_body(bytes, tag, what, "unexpected descriptor_tag")?;
    let (mask, rest) = body
        .split_first_chunk::<N>()
        .ok_or(Error::InvalidDescriptor {
            tag,
            reason: "descriptor body shorter than its address mask",
        })?;
    let (chunks, tail) = rest.as_chunks::<N>();
    Ok((
        A::from_octets(*mask),
        chunks.iter().map(|c| A::from_octets(*c)).collect(),
        tail,
    ))
}

/// Serialize `[mask][addr]*[trailing]` with the descriptor header.
pub(crate) fn write_masked<const N: usize, A: Octets<N>>(
    buf: &mut [u8],
    tag: u8,
    mask: A,
    addrs: &[A],
    trailing: &[u8],
) -> Result<usize> {
    check_trailing(tag, trailing, N)?;
    let len = HEADER_LEN + N + addrs.len() * N + trailing.len();
    if buf.len() < len {
        return Err(Error::OutputBufferTooSmall {
            need: len,
            have: buf.len(),
        });
    }
    write_descriptor_header(buf, tag, len - HEADER_LEN)?;
    buf[HEADER_LEN..HEADER_LEN + N].copy_from_slice(&mask.to_octets());
    let mut off = HEADER_LEN + N;
    for a in addrs {
        buf[off..off + N].copy_from_slice(&a.to_octets());
        off += N;
    }
    buf[off..off + trailing.len()].copy_from_slice(trailing);
    Ok(len)
}

/// Parse a pure entry list (`[entry]*[tail]`) after the descriptor header.
pub(crate) fn parse_list<'a, const E: usize, T: Entry<E>>(
    bytes: &'a [u8],
    tag: u8,
    what: &'static str,
) -> Result<(Vec<T>, &'a [u8])> {
    let body = descriptor_body(bytes, tag, what, "unexpected descriptor_tag")?;
    Ok(parse_entries::<E, T>(body))
}

/// Serialize a pure entry list with the descriptor header.
pub(crate) fn write_list<const E: usize, T: Entry<E>>(
    buf: &mut [u8],
    tag: u8,
    entries: &[T],
    trailing: &[u8],
) -> Result<usize> {
    check_trailing(tag, trailing, E)?;
    let len = HEADER_LEN + entries.len() * E + trailing.len();
    if buf.len() < len {
        return Err(Error::OutputBufferTooSmall {
            need: len,
            have: buf.len(),
        });
    }
    write_descriptor_header(buf, tag, len - HEADER_LEN)?;
    write_entries(buf, HEADER_LEN, entries, trailing);
    Ok(len)
}

/// Wire length of a pure entry list descriptor.
pub(crate) const fn list_len(entries: usize, entry_len: usize, trailing: usize) -> usize {
    HEADER_LEN + entries * entry_len + trailing
}
