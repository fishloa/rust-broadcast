//! Print every metadata item of an MP4/MOV file through the blocking driver,
//! then a summary block built from the typed accessors.
//!
//! ```bash
//! cargo run -p isobmff --example read_tags -- fixtures/mp4/progressive/av-prog.mp4
//! ```

use std::env;
use std::fs::File;

use isobmff::ReaderConfig;
use isobmff::driver::{DriverStep, FileReader};
use isobmff::metadata::{Metadata, MetadataItem, MovieMetadata};

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: read_tags <FILE>");
        std::process::exit(2);
    });
    let file = File::open(&path).expect("open the input file");
    let mut reader = FileReader::new(file, ReaderConfig::default()).expect("wrap the input file");
    let mut metadata = None;
    loop {
        match reader.step().expect("driver step") {
            DriverStep::Metadata(found) => {
                metadata = Some(found.clone());
                break;
            }
            DriverStep::Done => break,
            _ => {}
        }
    }
    match metadata {
        Some(metadata) => print(&metadata),
        None => println!("no metadata"),
    }
}

fn print(metadata: &MovieMetadata) {
    describe_scope("movie", &metadata.movie);
    for (track, track_metadata) in &metadata.tracks {
        describe_scope(&format!("track {track}"), track_metadata);
    }
    let movie = &metadata.movie;
    println!();
    if let Some(title) = movie.title() {
        println!("Title: {title}");
    }
    if let Some(artist) = movie.artist() {
        println!("Artist: {artist}");
    }
    if let Some(album) = movie.album() {
        println!("Album: {album}");
    }
    if let Some(date) = movie.date() {
        println!("Date: {date}");
    }
    if let Some(genre) = movie.genre() {
        println!("Genre: {genre}");
    }
    if let Some(comment) = movie.comment() {
        println!("Comment: {comment}");
    }
    if let Some(encoder) = movie.encoder() {
        println!("Encoder: {encoder}");
    }
    if let Some((index, total)) = movie.track() {
        println!("Track: {index}/{total}");
    }
    if let Some((index, total)) = movie.disc() {
        println!("Disc: {index}/{total}");
    }
    if let Some(art) = movie.cover_art() {
        println!(
            "Cover art: {} bytes of type {}",
            art.bytes.len(),
            art.data_type
        );
    }
    if let Some(location) = movie.location() {
        println!("Location: {}", location.as_str());
    }
    if !movie.chapters().is_empty() {
        println!("Chapters: {}", movie.chapters().len());
    }
}

fn describe_scope(scope: &str, metadata: &Metadata) {
    for item in &metadata.items {
        println!("{scope}: {}", describe(item));
    }
}

fn describe(item: &MetadataItem) -> String {
    match item {
        MetadataItem::Text { key, value, .. } => format!("{key} = {value:?}"),
        MetadataItem::Number { key, value, .. } => format!("{key} = {value}"),
        MetadataItem::Pair { key, index, total } => {
            format!("{key} = {index}/{total}")
        }
        MetadataItem::Binary {
            key,
            data_type,
            bytes,
            ..
        } => {
            format!("{key} = {} bytes of type {data_type}", bytes.len())
        }
        MetadataItem::Freeform { mean, name, values } => {
            format!("{mean}/{name} = {} value(s)", values.len())
        }
        MetadataItem::Location(location) => format!("location = {}", location.as_str()),
        MetadataItem::Loci(loci) => format!(
            "location = {:.4},{:.4}",
            loci.latitude_degrees(),
            loci.longitude_degrees()
        ),
        MetadataItem::Chapters(chapters) => format!("{} chapter(s)", chapters.len()),
        MetadataItem::Opaque(boxed) => {
            format!("{:?} = {} opaque bytes", boxed.box_type, boxed.data.len())
        }
        _ => "unknown item".into(),
    }
}
