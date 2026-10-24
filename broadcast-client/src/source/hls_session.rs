//! HLS session core: the main playlist adapter plus an optional second adapter for a separate
//! audio rendition, sharing one connection-id allocator, one [`AuthState`] and one event stream.

use alloc::collections::VecDeque;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use broadcast_hls::MediaType;
use hls_runtime::client::{MasterSelection, RenditionInfo};
use transmux::TrackSpec;
use url::Url;

use crate::auth::AuthState;
use crate::error::Error;
use crate::events::{Action, Config, Event, Status};
use crate::playout::Source;
use crate::source::hls::{ConnAlloc, HlsSource, Role};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioMode {
    Auto,
    Manual(Option<usize>),
}

struct Audio {
    source: HlsSource,
    url: String,
    ended: bool,
}

pub(crate) struct HlsSession {
    config: Config,
    alloc: ConnAlloc,
    main: HlsSource,
    audio: Option<Audio>,
    master: Option<MasterSelection>,
    mode: AudioMode,
    active: Option<(usize, RenditionInfo)>,
    main_tracks: Vec<TrackSpec>,
    audio_tracks: Vec<TrackSpec>,
    main_ended: bool,
    ended_sent: bool,
    establishing_sent: bool,
    live_sent: bool,
    session_fatal: bool,
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
}

impl HlsSession {
    pub(crate) fn new(url: &Url, config: &Config) -> Self {
        let alloc = ConnAlloc::new();
        let main = HlsSource::new(url, config, alloc.clone(), Role::Main);
        let mode = if config.auto_select_audio {
            AudioMode::Auto
        } else {
            AudioMode::Manual(None)
        };
        Self {
            config: config.clone(),
            alloc,
            main,
            audio: None,
            master: None,
            mode,
            active: None,
            main_tracks: Vec::new(),
            audio_tracks: Vec::new(),
            main_ended: false,
            ended_sent: false,
            establishing_sent: false,
            live_sent: false,
            session_fatal: false,
            actions: VecDeque::new(),
            events: VecDeque::new(),
        }
    }

    pub(crate) fn playout_source(&self) -> Source {
        if self.main.endlist_seen() {
            Source::Vod
        } else {
            Source::Live {
                target_duration_secs: self.main.target_duration_secs(),
            }
        }
    }

    pub(crate) fn take_actions(&mut self) -> VecDeque<Action> {
        core::mem::take(&mut self.actions)
    }

    pub(crate) fn take_events(&mut self) -> VecDeque<Event> {
        core::mem::take(&mut self.events)
    }

    pub(crate) fn conns(&self) -> Vec<u32> {
        let mut conns = self.main.conns();
        if let Some(a) = &self.audio {
            conns.extend(a.source.conns());
        }
        conns
    }

    pub(crate) fn poll(&mut self, now: u64, auth: &mut AuthState) {
        self.main.poll(now, auth);
        if let Some(a) = self.audio.as_mut() {
            a.source.poll(now, auth);
        }
        self.collect(now, auth);
    }

    pub(crate) fn on_response(
        &mut self,
        conn: u32,
        status: u16,
        headers: &[(String, String)],
        now: u64,
        auth: &mut AuthState,
    ) {
        match self.audio.as_mut().filter(|a| a.source.owns(conn)) {
            Some(a) => a.source.on_response(conn, status, headers, now, auth),
            None => self.main.on_response(conn, status, headers, now, auth),
        }
        self.collect(now, auth);
    }

    pub(crate) fn on_data(&mut self, conn: u32, bytes: &[u8], now: u64, auth: &mut AuthState) {
        match self.audio.as_mut().filter(|a| a.source.owns(conn)) {
            Some(a) => a.source.on_data(conn, bytes, now, auth),
            None => self.main.on_data(conn, bytes, now, auth),
        }
        self.collect(now, auth);
    }

    pub(crate) fn on_closed(
        &mut self,
        conn: u32,
        error: Option<&str>,
        now: u64,
        auth: &mut AuthState,
    ) {
        match self.audio.as_mut().filter(|a| a.source.owns(conn)) {
            Some(a) => a.source.on_closed(conn, error, now, auth),
            None => self.main.on_closed(conn, error, now, auth),
        }
        self.collect(now, auth);
    }

    pub(crate) fn resume(&mut self, now: u64, auth: &mut AuthState) {
        self.main.resume(now, auth);
        if let Some(a) = self.audio.as_mut() {
            a.source.resume(now, auth);
        }
        self.collect(now, auth);
    }

    pub(crate) fn reconnect(&mut self, now: u64, auth: &mut AuthState) {
        self.main.reconnect(now, auth);
        if let Some(a) = self.audio.as_mut() {
            a.source.reconnect(now, auth);
        }
        self.collect(now, auth);
    }

    pub(crate) fn select_variant(
        &mut self,
        index: usize,
        now: u64,
        auth: &mut AuthState,
    ) -> Result<(), Error> {
        self.main.select_variant(index, now, auth)?;
        if let Some(m) = self.master.as_mut() {
            m.chosen = index;
        }
        self.reevaluate_after_variant_change(now, auth);
        self.collect(now, auth);
        Ok(())
    }

    pub(crate) fn select_audio(
        &mut self,
        choice: Option<usize>,
        now: u64,
        auth: &mut AuthState,
    ) -> Result<(), Error> {
        let candidates = self.candidates();
        if self.master.is_none() || choice.is_some_and(|i| i >= candidates.len()) {
            return Err(Error::NoSuchRendition);
        }
        if self.ended_sent {
            return Ok(());
        }
        self.mode = AudioMode::Manual(choice);
        self.apply_audio(choice, now, auth);
        self.collect(now, auth);
        Ok(())
    }

    fn candidates(&self) -> Vec<RenditionInfo> {
        let Some(master) = &self.master else {
            return Vec::new();
        };
        let Some(group) = master
            .variants
            .get(master.chosen)
            .and_then(|v| v.audio_group.as_deref())
        else {
            return Vec::new();
        };
        master
            .renditions
            .iter()
            .filter(|r| r.media_type == MediaType::Audio && r.group_id == group)
            .cloned()
            .collect()
    }

    fn auto_pick(&self, candidates: &[RenditionInfo]) -> Option<usize> {
        if let Some(pref) = self.config.preferred_audio_language.as_deref()
            && let Some(i) = candidates.iter().position(|r| {
                (r.autoselect || r.default)
                    && r.language
                        .as_deref()
                        .is_some_and(|l| language_matches(l, pref))
            })
        {
            return Some(i);
        }
        candidates
            .iter()
            .position(|r| r.default)
            .or_else(|| candidates.iter().position(|r| r.url.is_some()))
    }

    fn reevaluate_after_variant_change(&mut self, now: u64, auth: &mut AuthState) {
        let candidates = self.candidates();
        let target = match self.mode {
            AudioMode::Manual(None) => None,
            AudioMode::Manual(Some(_)) => self
                .active
                .as_ref()
                .and_then(|(_, old)| {
                    candidates
                        .iter()
                        .position(|r| r.name == old.name && r.language == old.language)
                })
                .or_else(|| {
                    self.config
                        .auto_select_audio
                        .then(|| self.auto_pick(&candidates))
                        .flatten()
                }),
            AudioMode::Auto => self.auto_pick(&candidates),
        };
        self.apply_audio(target, now, auth);
    }

    fn apply_audio(&mut self, target: Option<usize>, now: u64, auth: &mut AuthState) {
        if self.session_fatal {
            return;
        }
        let candidates = self.candidates();
        let rendition = target.and_then(|i| candidates.get(i).cloned().map(|r| (i, r)));
        let main_url = self
            .master
            .as_ref()
            .and_then(|m| m.variants.get(m.chosen))
            .map(|v| v.url.clone());
        let want_url = rendition
            .as_ref()
            .and_then(|(_, r)| r.url.clone())
            .filter(|u| Some(u) != main_url.as_ref());
        if self.audio.as_ref().map(|a| &a.url) != want_url.as_ref() {
            self.stop_audio();
            if let Some(url) = want_url.as_deref().and_then(|u| Url::parse(u).ok()) {
                let source = HlsSource::new(&url, &self.config, self.alloc.clone(), Role::Audio);
                self.audio = Some(Audio {
                    source,
                    url: url.to_string(),
                    ended: false,
                });
            }
        }
        if rendition != self.active {
            self.active = rendition.clone();
            self.events.push_back(Event::AudioRendition {
                index: rendition.as_ref().map(|(i, _)| *i),
                name: rendition.as_ref().map(|(_, r)| r.name.clone()),
                language: rendition.as_ref().and_then(|(_, r)| r.language.clone()),
            });
        }
        if let Some(a) = self.audio.as_mut() {
            a.source.poll(now, auth);
        }
        self.maybe_end();
    }

    fn stop_audio(&mut self) {
        if let Some(mut a) = self.audio.take() {
            a.source.halt();
            self.actions.extend(a.source.take_actions());
        }
        if !self.audio_tracks.is_empty() {
            self.audio_tracks.clear();
            self.events
                .push_back(Event::Tracks(self.main_tracks.clone()));
        }
    }

    fn maybe_end(&mut self) {
        let audio_done = self.audio.as_ref().is_none_or(|a| a.ended);
        if self.main_ended && audio_done && !self.ended_sent {
            self.ended_sent = true;
            self.events.push_back(Event::Status(Status::Ended));
        }
    }

    fn merged_tracks(&self) -> Vec<TrackSpec> {
        let mut all = self.main_tracks.clone();
        all.extend(self.audio_tracks.iter().cloned());
        all
    }

    fn collect(&mut self, now: u64, auth: &mut AuthState) {
        loop {
            let mut progressed = false;
            self.actions.extend(self.main.take_actions());
            let main_events = self.main.take_events();
            progressed |= !main_events.is_empty();
            for e in main_events {
                if self.session_fatal {
                    break;
                }
                let fatal = is_fatal_error(&e);
                self.handle_main(e, now, auth);
                if fatal {
                    self.propagate_fatal();
                }
            }
            if let Some(a) = self.audio.as_mut() {
                self.actions.extend(a.source.take_actions());
                let audio_events = a.source.take_events();
                progressed |= !audio_events.is_empty();
                for e in audio_events {
                    if self.session_fatal {
                        break;
                    }
                    let fatal = is_fatal_error(&e);
                    self.handle_audio(e);
                    if fatal {
                        self.propagate_fatal();
                    }
                }
            }
            self.propagate_fatal();
            if !progressed {
                break;
            }
        }
        self.actions.extend(self.main.take_actions());
        if let Some(a) = self.audio.as_mut() {
            self.actions.extend(a.source.take_actions());
        }
    }

    fn propagate_fatal(&mut self) {
        if self.session_fatal {
            return;
        }
        let audio_fatal = self.audio.as_ref().is_some_and(|a| a.source.is_fatal());
        if self.main.is_fatal() || audio_fatal {
            self.session_fatal = true;
            self.main.halt();
            if let Some(a) = self.audio.as_mut() {
                a.source.halt();
            }
        }
    }

    fn handle_main(&mut self, event: Event, now: u64, auth: &mut AuthState) {
        match event {
            Event::Variants(selection) => {
                self.master = Some(selection.clone());
                self.events.push_back(Event::Variants(selection));
                self.reevaluate_after_variant_change(now, auth);
            }
            Event::Tracks(specs) => {
                self.main_tracks = specs;
                self.events.push_back(Event::Tracks(self.merged_tracks()));
            }
            Event::Status(Status::Ended) => {
                self.main_ended = true;
                self.maybe_end();
            }
            other => self.forward_common(other),
        }
    }

    fn handle_audio(&mut self, event: Event) {
        match event {
            Event::Variants(_) => {}
            Event::Tracks(specs) => {
                self.audio_tracks = specs;
                self.events.push_back(Event::Tracks(self.merged_tracks()));
            }
            Event::Status(Status::Ended) => {
                if let Some(a) = self.audio.as_mut() {
                    a.ended = true;
                }
                self.maybe_end();
            }
            other => self.forward_common(other),
        }
    }

    fn forward_common(&mut self, event: Event) {
        match event {
            Event::Status(Status::Establishing) => {
                if !self.establishing_sent {
                    self.establishing_sent = true;
                    self.events.push_back(event);
                }
            }
            Event::Status(Status::Live) => {
                if !self.live_sent {
                    self.live_sent = true;
                    self.events.push_back(event);
                }
            }
            other => self.events.push_back(other),
        }
    }
}

fn is_fatal_error(event: &Event) -> bool {
    matches!(event, Event::Error { fatal: true, .. })
}

fn language_matches(language: &str, preferred: &str) -> bool {
    let language = language.to_ascii_lowercase();
    let preferred = preferred.to_ascii_lowercase();
    language == preferred
        || language
            .strip_prefix(preferred.as_str())
            .is_some_and(|rest| rest.starts_with('-'))
}
