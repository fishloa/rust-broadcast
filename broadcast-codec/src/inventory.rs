//! One line of the run-time inventory, and the table it renders as.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::availability::Probe;
use crate::codec::token;
use crate::descriptor::CodecDescriptor;
use crate::provider_info::ProviderInfo;

/// The number of columns the table writes.
const COLUMNS: usize = 9;
/// What separates two table columns.
const COLUMN_SEPARATOR: &str = "  ";
/// What a cell holds when the descriptor or provider states nothing.
const ABSENT: &str = "-";
/// The table's header row.
const HEADER: [&str; COLUMNS] = [
    "codec",
    "direction",
    "provider",
    "implementation",
    "kind",
    "lossless",
    "profiles",
    "availability",
    "licence",
];

/// One implementation in the run-time inventory.
///
/// It carries the [`CodecDescriptor`], the [`ProviderInfo`] of the provider
/// that offers it, and the [`Probe`] answering whether it opens here.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InventoryEntry {
    descriptor: CodecDescriptor,
    provider: ProviderInfo,
    probe: Probe,
}

impl InventoryEntry {
    /// An entry built from a descriptor, its provider's information and a
    /// probe answer.
    #[must_use]
    pub fn new(descriptor: CodecDescriptor, provider: ProviderInfo, probe: Probe) -> Self {
        Self {
            descriptor,
            provider,
            probe,
        }
    }

    /// The implementation this entry describes.
    #[must_use]
    pub const fn descriptor(&self) -> &CodecDescriptor {
        &self.descriptor
    }

    /// The information the offering provider reports.
    #[must_use]
    pub const fn provider(&self) -> &ProviderInfo {
        &self.provider
    }

    /// Whether the implementation opens here.
    #[must_use]
    pub const fn probe(&self) -> &Probe {
        &self.probe
    }
}

impl fmt::Display for InventoryEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let descriptor = &self.descriptor;
        write!(
            f,
            "{} {} {} {} {}",
            token(descriptor.codec()),
            descriptor.direction(),
            descriptor.provider(),
            descriptor.implementation(),
            kind(descriptor),
        )?;
        if descriptor.lossless() {
            f.write_str(" lossless")?;
        }
        write!(
            f,
            " {} {} {}",
            profiles(descriptor),
            self.probe.availability(),
            self.provider.licence().unwrap_or(ABSENT),
        )
    }
}

/// Render `entries` as a table with one header row and one row per entry.
///
/// Column widths are computed from the header and the entries, columns are
/// separated by two spaces and every line ends in a newline. Nothing depends
/// on a terminal and nothing is coloured.
#[must_use]
pub fn render_inventory(entries: &[InventoryEntry]) -> String {
    let rows: Vec<[String; COLUMNS]> = entries.iter().map(row).collect();
    let mut widths = [0; COLUMNS];
    for (index, cell) in HEADER.iter().enumerate() {
        widths[index] = cell.len();
    }
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.len());
        }
    }
    let mut table = String::new();
    write_line(&mut table, &HEADER, &widths);
    for row in &rows {
        let cells: [&str; COLUMNS] = core::array::from_fn(|index| row[index].as_str());
        write_line(&mut table, &cells, &widths);
    }
    table
}

/// One entry as its table cells.
fn row(entry: &InventoryEntry) -> [String; COLUMNS] {
    let descriptor = entry.descriptor();
    [
        token(descriptor.codec()),
        descriptor.direction().name().into(),
        descriptor.provider().into(),
        descriptor.implementation().into(),
        kind(descriptor).into(),
        lossless(descriptor).into(),
        profiles(descriptor),
        entry.probe().availability().name().into(),
        entry.provider().licence().unwrap_or(ABSENT).into(),
    ]
}

/// Write one padded, right-trimmed line.
fn write_line(table: &mut String, cells: &[&str; COLUMNS], widths: &[usize; COLUMNS]) {
    let mut line = String::new();
    for (index, (cell, width)) in cells.iter().zip(widths).enumerate() {
        if index > 0 {
            line.push_str(COLUMN_SEPARATOR);
        }
        line.push_str(cell);
        for _ in cell.len()..*width {
            line.push(' ');
        }
    }
    while line.ends_with(' ') {
        line.pop();
    }
    line.push('\n');
    table.push_str(&line);
}

/// `hw` for a dedicated-hardware implementation, `sw` otherwise.
fn kind(descriptor: &CodecDescriptor) -> &'static str {
    if descriptor.hardware() { "hw" } else { "sw" }
}

/// `yes` for a lossless implementation, [`ABSENT`] otherwise.
fn lossless(descriptor: &CodecDescriptor) -> &'static str {
    if descriptor.lossless() { "yes" } else { ABSENT }
}

/// The profiles joined by `/`, or [`ABSENT`].
fn profiles(descriptor: &CodecDescriptor) -> String {
    if descriptor.profiles().is_empty() {
        ABSENT.into()
    } else {
        descriptor.profiles().join("/")
    }
}
