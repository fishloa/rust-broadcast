//! RTCP control packets (RFC 3550 §6): SR/RR/SDES/BYE/APP and the §6.1
//! compound packet. Absorbed from the former `rtcp-packet` crate.

mod error;
mod packet;

pub use error::{Error, PaddingCountError, Result};
pub use packet::{
    APP_NAME_LEN, App, Bye, CommonHeader, CompoundPacket, PT_APP, PT_BYE, PT_RECEIVER_REPORT,
    PT_SENDER_REPORT, PT_SOURCE_DESCRIPTION, REPORT_BLOCK_LEN, ReceiverReport, ReportBlock,
    RtcpPacket, RtcpPacketType, SDES_CNAME, SDES_EMAIL, SDES_LOC, SDES_NAME, SDES_NOTE, SDES_PHONE,
    SDES_PRIV, SDES_TOOL, SdesChunk, SdesItem, SdesItemType, SenderReport, SourceDescription,
    padding_count,
};
