//! Unified command dispatch: [`AnyCommand`].
//!
//! [`AnyCommand`] is generated from a single declarative list
//! (`declare_commands!`) — one line per `splice_command_type` (§9.6.1,
//! Table 7). The list is the single source of truth: it produces the enum, the
//! `From<T>` conversions, the type → parser dispatcher, and a drift test that
//! pins each command-type literal to the type's
//! [`CommandDef::COMMAND_TYPE`](crate::traits::CommandDef::COMMAND_TYPE).
//!
//! A `splice_command_type` with no typed implementation (the reserved values)
//! falls through to [`AnyCommand::Unknown`], which keeps the raw command body
//! so a section round-trips byte-for-byte.

use crate::error::Result;

/// Declares [`AnyCommand`] + its dispatcher from one command-type list.
macro_rules! declare_commands {
    (
        $lt:lifetime;
        $( $variant:ident = $ct:literal => $($path:ident)::+ $(<$plt:lifetime>)? ),+ $(,)?
    ) => {
        /// Every crate-implemented splice command, plus an `Unknown`
        /// fallthrough that preserves the raw command body for lossless
        /// round-trips.
        ///
        /// serde uses external tagging with camelCase variant keys.
        #[derive(Debug, Clone, PartialEq, Eq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
        #[non_exhaustive]
        pub enum AnyCommand<$lt> {
            $(
                #[allow(missing_docs)]
                $variant($($path)::+ $(<$plt>)?),
            )+
            /// A `splice_command_type` with no typed implementation; `body` is
            /// the raw command bytes (`splice_command_length` bytes).
            Unknown {
                /// The raw `splice_command_type` byte.
                command_type: u8,
                /// The raw command body bytes.
                body: &$lt [u8],
            },
        }

        $(
            impl<$lt> From<$($path)::+ $(<$plt>)?> for AnyCommand<$lt> {
                fn from(c: $($path)::+ $(<$plt>)?) -> Self {
                    Self::$variant(c)
                }
            }
        )+

        impl<$lt> AnyCommand<$lt> {
            /// Every `splice_command_type` the generated dispatcher routes
            /// (excludes [`AnyCommand::Unknown`]).
            pub const DISPATCHED_TYPES: &'static [u8] = &[$($ct),+];

            /// Diagnostic name of the contained command — the type's
            /// [`CommandDef::NAME`](crate::traits::CommandDef::NAME)
            /// (`"SPLICE_INSERT"`, `"TIME_SIGNAL"`, …); `"UNKNOWN"` for
            /// [`AnyCommand::Unknown`].
            #[must_use]
            pub fn name(&self) -> &'static str {
                match self {
                    $(
                        Self::$variant(_) =>
                            <$($path)::+ as crate::traits::CommandDef>::NAME,
                    )+
                    Self::Unknown { .. } => "UNKNOWN",
                }
            }

            /// The wire `splice_command_type` byte for this command.
            #[must_use]
            pub fn command_type(&self) -> u8 {
                match self {
                    $(
                        Self::$variant(_) =>
                            <$($path)::+ as crate::traits::CommandDef>::COMMAND_TYPE,
                    )+
                    Self::Unknown { command_type, .. } => *command_type,
                }
            }

            /// Parse a command `body` by its `splice_command_type`. Reserved /
            /// unimplemented types yield [`AnyCommand::Unknown`].
            pub fn dispatch(command_type: u8, body: &$lt [u8]) -> Result<Self> {
                use dvb_common::Parse;
                match command_type {
                    $(
                        $ct => <$($path)::+>::parse(body).map(Self::$variant),
                    )+
                    _ => Ok(Self::Unknown { command_type, body }),
                }
            }

            /// Number of bytes [`serialize_body_into`](Self::serialize_body_into)
            /// will write (the `splice_command_length`).
            #[must_use]
            pub fn body_len(&self) -> usize {
                use dvb_common::Serialize;
                match self {
                    $(
                        Self::$variant(c) => c.serialized_len(),
                    )+
                    Self::Unknown { body, .. } => body.len(),
                }
            }

            /// Serialize just the command body (no type byte) into `buf`.
            pub fn serialize_body_into(&self, buf: &mut [u8]) -> Result<usize> {
                use dvb_common::Serialize;
                match self {
                    $(
                        Self::$variant(c) => c.serialize_into(buf),
                    )+
                    Self::Unknown { body, .. } => {
                        if buf.len() < body.len() {
                            return Err(crate::error::Error::OutputBufferTooSmall {
                                need: body.len(),
                                have: buf.len(),
                            });
                        }
                        buf[..body.len()].copy_from_slice(body);
                        Ok(body.len())
                    }
                }
            }
        }

        #[cfg(test)]
        mod macro_drift {
            #[test]
            fn command_type_literals_match_command_def() {
                use crate::traits::CommandDef;
                $(
                    assert_eq!(
                        $ct,
                        <$($path)::+ as CommandDef>::COMMAND_TYPE,
                        concat!("command_type literal drift for ", stringify!($variant)),
                    );
                    assert!(
                        !<$($path)::+ as CommandDef>::NAME.is_empty(),
                        concat!("empty NAME for ", stringify!($variant)),
                    );
                )+
            }
        }
    };
}

declare_commands! {'a;
    SpliceNull           = 0x00 => crate::commands::splice_null::SpliceNull,
    SpliceSchedule       = 0x04 => crate::commands::splice_schedule::SpliceSchedule,
    SpliceInsert         = 0x05 => crate::commands::splice_insert::SpliceInsert,
    TimeSignal           = 0x06 => crate::commands::time_signal::TimeSignal,
    BandwidthReservation = 0x07 => crate::commands::bandwidth_reservation::BandwidthReservation,
    PrivateCommand       = 0xFF => crate::commands::private_command::PrivateCommand<'a>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_command_type_round_trips_body() {
        let body = [0xDE, 0xAD, 0xBE, 0xEF];
        let cmd = AnyCommand::dispatch(0x03, &body).unwrap();
        assert!(matches!(
            cmd,
            AnyCommand::Unknown {
                command_type: 0x03,
                ..
            }
        ));
        assert_eq!(cmd.body_len(), 4);
        assert_eq!(cmd.command_type(), 0x03);
        assert_eq!(cmd.name(), "UNKNOWN");
        let mut buf = vec![0u8; cmd.body_len()];
        cmd.serialize_body_into(&mut buf).unwrap();
        assert_eq!(buf, body);
    }

    #[test]
    fn dispatch_splice_null() {
        let cmd = AnyCommand::dispatch(0x00, &[]).unwrap();
        assert_eq!(cmd.name(), "SPLICE_NULL");
        assert_eq!(cmd.command_type(), 0x00);
        assert_eq!(cmd.body_len(), 0);
    }
}
