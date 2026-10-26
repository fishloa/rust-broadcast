//! Parse an MPD file and print its Periods, Representations and the first
//! segment URLs of each, exactly as a player would address them.
//!
//! Static addressing only needs the MPD; a `SegmentBase` (on-demand)
//! Representation also needs its `sidx`, which this example reads from the
//! file next to the MPD.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p dash-runtime --example list_segments
//! cargo run -p dash-runtime --example list_segments -- --mpd fixtures/dash/timeline-gpac/timeline.mpd --first 3
//! ```

use std::path::PathBuf;

use dash_runtime::address::{
    AddressError, PlanError, PlanOptions, base_index, plan_representation,
};
use dash_runtime::mpd::Mpd;
use dash_runtime::mpd::inherit::{SegmentInfo, resolve_segment_info};
use dash_runtime::selection::RepRef;
use url::Url;

const DEFAULT_MPD: &str = "../fixtures/dash/manifest.mpd";
const DEFAULT_FIRST: usize = 3;

struct Args {
    mpd: PathBuf,
    first: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        mpd: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_MPD),
        first: DEFAULT_FIRST,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--mpd" => args.mpd = it.next().ok_or("--mpd needs a path")?.into(),
            "--first" => {
                args.first = it
                    .next()
                    .ok_or("--first needs a number")?
                    .parse()
                    .map_err(|e| format!("--first: {e}"))?;
            }
            other => {
                return Err(format!(
                    "unknown argument {other} (use --mpd PATH, --first N)"
                ));
            }
        }
    }
    Ok(args)
}

fn sidx_of(
    mpd: &Mpd,
    at: RepRef,
    doc: &Url,
) -> Result<(transmux::SegmentIndexBox, u64), Box<dyn std::error::Error>> {
    let (period, set, rep) = at.resolve(mpd).ok_or("no such Representation")?;
    let SegmentInfo::Base(base) = resolve_segment_info(period, set, rep)? else {
        return Err("not a SegmentBase Representation".into());
    };
    let range = base.index_range.ok_or("SegmentBase without indexRange")?;
    let cands = dash_runtime::mpd::urls::representation_candidates(mpd, period, set, rep);
    let file = Url::parse(
        &cands[0]
            .resolve(Some(doc), "")
            .ok_or("unresolvable BaseURL")?,
    )?
    .to_file_path()
    .map_err(|()| "BaseURL is not a file")?;
    let bytes = std::fs::read(file)?;
    let (a, b) = (
        range.first as usize,
        range.last.ok_or("open-ended range")? as usize,
    );
    let sidx = base_index::parse_index(bytes.get(a..=b).ok_or("indexRange outside the file")?)?;
    Ok((sidx, base_index::anchor_after(&range)?))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let path = std::fs::canonicalize(&args.mpd)?;
    let doc = Url::from_file_path(&path).map_err(|()| "MPD path is not absolute")?;
    let mpd = Mpd::parse(&std::fs::read(&path)?)?;
    println!(
        "{} ({}), {} period(s)",
        path.display(),
        mpd.kind,
        mpd.periods.len()
    );
    for (pi, period) in mpd.periods.iter().enumerate() {
        println!("Period {} ({})", pi, period.id.as_deref().unwrap_or("-"));
        for (si, set) in period.adaptation_sets.iter().enumerate() {
            for (ri, rep) in set.representations.iter().enumerate() {
                let at = RepRef::at(pi, si, ri);
                let mut planned = plan_representation(&mpd, at, &PlanOptions::default());
                let sidx;
                if let Err(PlanError::Address(AddressError::IndexRequired)) = &planned {
                    sidx = sidx_of(&mpd, at, &doc)?;
                    planned = plan_representation(
                        &mpd,
                        at,
                        &PlanOptions::default().with_index(&sidx.0, sidx.1),
                    );
                }
                match planned {
                    Err(e) => println!("  rep {} (set {si}): dropped: {e}", rep.id),
                    Ok(p) => {
                        let count = p
                            .plan
                            .len()
                            .map_or("open-ended".to_owned(), |n| n.to_string());
                        println!(
                            "  rep {} bandwidth {} segments {count}",
                            rep.id, rep.bandwidth
                        );
                        if let Some(init) = p.plan.init() {
                            let url = p.candidates[0].resolve(Some(&doc), &init.reference);
                            println!("    init   {}", url.unwrap_or_default());
                        }
                        for seg in p.plan.iter().take(args.first) {
                            let url = p.candidates[0].resolve(Some(&doc), &seg.reference);
                            let range = seg
                                .byte_range
                                .map(|r| format!(" bytes {}-{}", r.first, r.last.unwrap_or(0)))
                                .unwrap_or_default();
                            println!(
                                "    #{:<6} t={:>10.3}s d={:>7.3}s {}{range}",
                                seg.number,
                                seg.start_us as f64 / 1e6,
                                seg.duration_us as f64 / 1e6,
                                url.unwrap_or_default()
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
