//! Virtual filesystem view of a DVB object carousel.
//!
//! [`CarouselFs`] is built from a set of `(module_id, &[u8])` pairs (the
//! reassembled module data from [`crate::carousel::ModuleReassembler`]).
//! It walks each module's BIOP messages, indexes by `(module_id, object_key)`,
//! and exposes a path-based resolver so callers can retrieve file content
//! without understanding the IOR chain.
//!
//! Spec: `docs/iso_13818_6_biop.md` (ETSI TR 101 202 §4.7.4).

use super::message::{BiopMessage, DirectoryMessage};
use std::collections::HashMap;

// ── CarouselObject ────────────────────────────────────────────────────────────

/// One object parsed from a carousel module.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CarouselObject {
    /// A Directory or ServiceGateway object.
    Directory(DirectoryObjectData),
    /// A File object.
    File(FileObjectData),
}

/// Owned data extracted from a `DirectoryMessage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryObjectData {
    /// Bindings in this directory: `(name_bytes_no_nul, module_id, object_key_bytes)`.
    pub entries: Vec<(Vec<u8>, u16, Vec<u8>)>,
    /// True if this is a ServiceGateway (the carousel root).
    pub is_service_gateway: bool,
}

/// Owned data extracted from a `FileMessage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileObjectData {
    /// File content bytes.
    pub content: Vec<u8>,
}

// ── Key type ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ObjectKey {
    module_id: u16,
    object_key: Vec<u8>,
}

// ── CarouselFs ────────────────────────────────────────────────────────────────

/// A virtual filesystem built from a set of reassembled carousel modules.
///
/// # Construction
///
/// ```text
/// // collect (module_id, data) pairs from ModuleReassembler:
/// let fs = CarouselFs::from_modules(&[(1, data_bytes)]);
/// ```
///
/// # Path resolution
///
/// Paths are `&[&str]` slices (e.g. `&["images", "logo.png"]`).
/// The root is the ServiceGateway object; each step follows a binding name.
/// Binding names that end in `\0` have the trailing NUL stripped before matching.
#[derive(Debug, Clone)]
pub struct CarouselFs {
    objects: HashMap<ObjectKey, CarouselObject>,
    /// Key of the ServiceGateway (root) object, if found.
    root_key: Option<ObjectKey>,
}

impl CarouselFs {
    /// Build a `CarouselFs` from module `(module_id, data)` pairs.
    ///
    /// Each module's bytes are walked via `BiopMessage::parse_at`.
    /// Unknown message kinds are silently skipped.
    pub fn from_modules(modules: &[(u16, &[u8])]) -> Self {
        let mut objects: HashMap<ObjectKey, CarouselObject> = HashMap::new();
        let mut root_key: Option<ObjectKey> = None;

        for &(module_id, data) in modules {
            let mut pos = 0;
            while pos < data.len() {
                let remaining = &data[pos..];
                match BiopMessage::parse_at(remaining) {
                    Ok((msg, consumed)) => {
                        // Extract the object key and kind from the message.
                        let (obj_key_bytes, obj) = extract_object(module_id, &msg);
                        if let Some(obj) = obj {
                            let is_sg = matches!(&msg, BiopMessage::ServiceGateway(_));
                            let key = ObjectKey {
                                module_id,
                                object_key: obj_key_bytes,
                            };
                            if is_sg && root_key.is_none() {
                                root_key = Some(key.clone());
                            }
                            objects.insert(key, obj);
                        }
                        pos += consumed;
                    }
                    Err(_) => break,
                }
            }
        }

        CarouselFs { objects, root_key }
    }

    /// Return the ServiceGateway (root) object, if present.
    pub fn service_gateway(&self) -> Option<&CarouselObject> {
        self.root_key.as_ref().and_then(|k| self.objects.get(k))
    }

    /// Resolve a path `&[&str]` starting from the ServiceGateway root.
    /// Returns the `CarouselObject` at that path, or `None`.
    pub fn resolve(&self, path: &[&str]) -> Option<&CarouselObject> {
        let mut cur_key = self.root_key.clone()?;
        for &segment in path {
            let dir = match self.objects.get(&cur_key)? {
                CarouselObject::Directory(d) => d,
                CarouselObject::File(_) => return None,
            };
            // Find binding with name matching `segment` (strip trailing NUL).
            let (_, mod_id, key_bytes) = dir.entries.iter().find(|(name, _, _)| {
                let n = strip_nul(name);
                n == segment.as_bytes()
            })?;
            cur_key = ObjectKey {
                module_id: *mod_id,
                object_key: key_bytes.clone(),
            };
        }
        self.objects.get(&cur_key)
    }

    /// Resolve a path and return the file content bytes, if the target is a File.
    pub fn file_bytes(&self, path: &[&str]) -> Option<&[u8]> {
        match self.resolve(path)? {
            CarouselObject::File(f) => Some(&f.content),
            CarouselObject::Directory(_) => None,
        }
    }
}

/// Strip a trailing NUL byte from a name slice.
fn strip_nul(name: &[u8]) -> &[u8] {
    if name.last() == Some(&0) {
        &name[..name.len() - 1]
    } else {
        name
    }
}

/// Extract the object key bytes and a `CarouselObject` from a parsed message.
/// Returns `(object_key_bytes, Some(obj))` or `(vec![], None)` if not indexable.
fn extract_object(module_id: u16, msg: &BiopMessage<'_>) -> (Vec<u8>, Option<CarouselObject>) {
    match msg {
        BiopMessage::Directory(dm) | BiopMessage::ServiceGateway(dm) => {
            let key_bytes = dm.object_key.to_vec();
            let entries = extract_dir_entries(module_id, dm);
            let is_sg = matches!(msg, BiopMessage::ServiceGateway(_));
            (
                key_bytes,
                Some(CarouselObject::Directory(DirectoryObjectData {
                    entries,
                    is_service_gateway: is_sg,
                })),
            )
        }
        BiopMessage::File(fm) => {
            let key_bytes = fm.object_key.to_vec();
            let content = fm.content.to_vec();
            (
                key_bytes,
                Some(CarouselObject::File(FileObjectData { content })),
            )
        }
        BiopMessage::Stream(_) | BiopMessage::StreamEvent(_) => (vec![], None),
    }
}

/// Extract binding entries from a DirectoryMessage as `(name, module_id, object_key)`.
fn extract_dir_entries(
    _self_module_id: u16,
    dm: &DirectoryMessage<'_>,
) -> Vec<(Vec<u8>, u16, Vec<u8>)> {
    let mut entries = Vec::with_capacity(dm.bindings.len());
    for binding in &dm.bindings {
        // DVB: nameComponents_count == 1 per binding.
        let name = binding
            .name
            .first()
            .map(|nc| nc.id.to_vec())
            .unwrap_or_default();
        // Get the module_id and object_key from the IOR BIOP profile.
        if let Some(bp) = binding.ior.biop_profile() {
            let mod_id = bp.object_location.module_id;
            let obj_key = bp.object_location.object_key.to_vec();
            entries.push((name, mod_id, obj_key));
        }
    }
    entries
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::carousel::biop::ior::NameComponent;
    use crate::carousel::biop::{
        ior::{BiopProfileBody, ConnBinder, Ior, ObjectLocation, TaggedProfile},
        message::{Binding, BiopMessage, DirectoryMessage, FileMessage},
        BINDING_NOBJECT,
    };
    use dvb_common::Serialize;

    /// Build a simple carousel in memory:
    ///   Module 1: ServiceGateway dir with one binding "index.html" → module 2, key [2]
    ///   Module 2: File with content b"hello world"
    fn build_test_carousel() -> Vec<(u16, Vec<u8>)> {
        // Build the IOR pointing to module 2, key [0x02]
        let file_ior = Ior {
            type_id: b"fil\0",
            profiles: vec![TaggedProfile::Biop(BiopProfileBody {
                object_location: ObjectLocation {
                    carousel_id: 0xAB,
                    module_id: 2,
                    version_major: 1,
                    version_minor: 0,
                    object_key: &[0x02],
                },
                conn_binder: ConnBinder { taps: vec![] },
                extra: vec![],
            })],
        };

        let sgw = BiopMessage::ServiceGateway(DirectoryMessage {
            object_kind: *b"srg\0",
            object_key: &[0x01],
            object_info: &[],
            service_context: vec![],
            bindings: vec![Binding {
                name: vec![NameComponent {
                    id: b"index.html",
                    kind: b"fil\0",
                }],
                binding_type: BINDING_NOBJECT,
                ior: file_ior,
                object_info: &[],
            }],
        });

        let file = BiopMessage::File(FileMessage {
            object_key: &[0x02],
            content_size: 11,
            object_info_extra: &[],
            service_context: vec![],
            content: b"hello world",
        });

        let mut mod1 = vec![0u8; sgw.serialized_len()];
        sgw.serialize_into(&mut mod1).unwrap();

        let mut mod2 = vec![0u8; file.serialized_len()];
        file.serialize_into(&mut mod2).unwrap();

        vec![(1u16, mod1), (2u16, mod2)]
    }

    #[test]
    fn carousel_fs_resolve_file() {
        let modules = build_test_carousel();
        let refs: Vec<(u16, &[u8])> = modules
            .iter()
            .map(|(id, data)| (*id, data.as_slice()))
            .collect();
        let fs = CarouselFs::from_modules(&refs);

        // Service gateway should be present
        assert!(fs.service_gateway().is_some());

        // File lookup
        let content = fs.file_bytes(&["index.html"]);
        assert_eq!(content, Some(b"hello world".as_slice()));
    }

    #[test]
    fn carousel_fs_resolve_missing_returns_none() {
        let modules = build_test_carousel();
        let refs: Vec<(u16, &[u8])> = modules
            .iter()
            .map(|(id, data)| (*id, data.as_slice()))
            .collect();
        let fs = CarouselFs::from_modules(&refs);
        assert!(fs.file_bytes(&["does-not-exist.html"]).is_none());
    }
}
