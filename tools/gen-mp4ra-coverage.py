#!/usr/bin/env python3
"""Regenerate isobmff's MP4RA coverage table.

Reads every CSV under `private/specs-verified/mpeg/isobmff/mp4ra/` (the
private checkout), classifies each code against the crate's typed sets, and
writes `isobmff/src/coverage/mp4ra.rs`. The pinned output is what the tests
read, so this is a maintenance tool, not a build step.

The classification sets below are edited by hand when a code is typed or a held
document is added; the CSV rows themselves are never copied into the public
tree. Run `python3 tools/gen-mp4ra-coverage.py` from anywhere; it rewrites
`isobmff/src/coverage/mp4ra.rs` from the private checkout, and `cargo fmt --all`
must follow to keep the table rustfmt-clean.
"""
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PRIVATE = ROOT / "private"
import csv,glob,re

def unesc(c): return c.replace('$20',' ')
TABLES=['aux-info-types','boxes','boxes-qt','boxes-udta','brands','checksum-types','color-types','data-references','entity-groups','handlers','item-properties','item-references','item-types','key-namespaces','knownduplicates','multiview-attributes','oti','sample-entries','sample-entries-boxes','sample-entries-qt','sample-groups','schemes','stream-types','textualcontent','track-groups','track-references','track-references-qt','track-selection','uncv-profiles','unlisted']
rows={}
for t in TABLES:
    p=PRIVATE/f'specs-verified/mpeg/isobmff/mp4ra/{t}.csv'
    if t=='knownduplicates':
        rows[t]=[]; continue
    rs=list(csv.DictReader(open(p,newline='')))
    # dedupe on code, keep first spec
    seen={}
    for r in rs:
        c=unesc(r['code']).rstrip()  # canonical code (trailing spaces stripped)
        if c not in seen: seen[c]=r
    rows[t]=[(c,r) for c,r in seen.items()]

# typed global sets -----------------------------------------------------------------
def read_typed_boxes():
    s=open(ROOT/'isobmff/src/coverage.rs').read()
    return set(m.group(1).rstrip() for m in re.finditer(r'p_\w+: b"([ -~]{4})"',s))
TB=read_typed_boxes()
TYPED={
 'Box': set(TB) | {'ftyp','styp','mvex','emsg','senc','pssh','tenc','sthd','ccst','md5i','url','wide','tran'},
 'QtAtom': {'albm','auth','cprt','elng'},
 'UdtaAtom': {'albm','alou','auth','cprt','dscp','gnre','hinf','hnti','kind','loci','ludt','perf','strk','titl','tlou','tsel','yrrc'},
 'SampleEntry': {'avc1','avc2','avc3','avc4','avcp','hvc1','hev1','dvh1','dvhe','vvc1','vvi1','mp4v','mp4a','enca','encv','ac-3','ec-3','ac-4','stpp','wvtt','av01','vp09','Opus','fLaC','mha1','mha2','mhm1','mhm2','dtsc','dtsh','dtsl','dtse','tmcd','metx','mett','stxt','urim','mebx','rgan','avs3','lav3','c608','lpcm','in24','in32','sowt','twos','raw','fl32','fl64','ima4','ulaw','alaw','NONE','rtp','srtp','rrtp','rsrp','rtcp','stcp','prtp','rm2t','sm2t','pm2t','fdp'},
 'SampleEntryBox': {'avcC','hvcC','vvcC','av1C','vpcC','dac3','dec3','dac4','dOps','dfLa','ddts','mhaC','dca3','av2c','la2c','lavc','av3c','esds','btrt','pasp','clap','colr','clli','mdcv','chnl','dmix','amve','cclv','srat','dvcC','dvvC','txtC','uriI','vttC','scrb','ccst'},
 'ItemProperty': {'vvcC','ispe','pasp','colr','pixi','rloc','auxC','clap','irot','lsel','imir','hvcC','avcC','av1C','a1op','a1lx','mdcv','clli','brnd','hdlp','scrb','amve','cclv','iscl','rref','crtt','mdft','udes','altt','aebr','wbbr','fobr','afbr','dobr','pano','tols','wipe','zoom','fade','splt','stpe','ssld','prdi','sstr','cmex','cmin','cexg','dadj','stag','reve','ndwt','mskC','subs','stvi'},
 'ItemType': {'hvc1','avc1','av01','vvc1','evc1','hvt1','lhv1','jpeg','auvd','grid','iovl','iden','tmap','cfen','sato','Exif','mime','uri','fdel','it35','mint','mski','rgan'},
 'Reference': {'thmb','auxl','dimg','cdsc','base','tbas','exbl','iloc','font','fdel','mint','pred','prem','dpnd','mask','eroi','drgn','rpds','anot'},
 'EntityGroup': {'altr','prsl','ster','eqiv','brst','tsyn','iaug','stem','aebr','wbbr','fobr','afbr','dobr','albc','favc','pano','slid','prgr','opeg','pymd','rgpa'},
 'SampleGroup': set(re.findall(r'\(b"([ -~]{4})"\)', open('.delegate/scratch/registered_codes.txt').read())) if os.path.exists('.delegate/scratch/registered_codes.txt') else set(),
 'TrackGroup': {'msrc','ster'},
 'TrackReference': {'adda','adrc','auxl','cdsc','font','hind','hint','subt','thmb','vdep','vplx','aest','avcp','deps','oref','sabt','sbas','scal','swfr','swto','tbas','tmcd','cdep','chap','scpt','ssrc'},
 'TrackSelection': {'bitr','bwas','cdec','cgsc','fgsc','frar','lang','mela','mpsz','mtyp','nvws','resc','scsz','spsc','tesc','tile','vwsc'},
 'KeyNamespace': {'mdta','me4c','udta','uiso'},
 'Scheme': {'spkt','essg','stvi','csch','cenc','cbcs'},
 'AuxInfoType': {'cenc','cbcs'},
 'ColourType': {'nclx','rICC','prof'},
 'DataReference': {'url','urn','imdt','snim'},
 'Brand': {'avc1','comp','iso2','iso3','iso4','iso5','iso6','iso7','iso9','isoa','isob','isoc','isod','isom','mp71','relo','saie','dash','dsms','emsg','lmsg','msdh','msix','risx','sims','sisx','ssss','avci','avcs','evbi','evbs','evmi','evms','hevc','hevm','hevs','hevx','jpeg','jpgs','vvic','vvis','MA1A','MA1B','qt','av01','3ge6','3ge7','3ge9','3gf9','3gg6','3gg9','3gh9','3gm9','3gp4','3gp5','3gp6','3gp7','3gp8','3gp9','3gr6','3gr9','3gs6','3gs9','3gt8','3gt9'},
 'ImageBrand': {'mif1','mif2','pred','1pic','tmap','msf1','iso8','unif','heic','heix','heim','heis','avif','avio','avis'},
 'Handler': {'3gsd','auxv','fdsm','hint','meta','mp7b','mp7t','null','soun','subt','vide','volv','hapt','mpd','mpdl','text','pict'},
}
# parse sample group codes from generated enum block
reg=open(ROOT/'isobmff/src/registered.rs').read()
sg=reg.split('SampleGroupType {')[1].split('}\n')[0]
TYPED['SampleGroup']=set(m.group(1).rstrip() for m in re.finditer(r'=> b"([ -~]{4})"',sg))
# likewise recompute all enum sets from registered.rs to avoid drift
for ty,key in [('Brand','Brand'),('HandlerType','Handler'),('TrackReferenceType','TrackReference'),('TrackGroupType','TrackGroup'),('TrackSelectionType','TrackSelection'),('KeyNamespace','KeyNamespace'),('RestrictedSchemeType','Scheme')]:
    blk=reg.split(ty+' {')[1].split('}\n')[0]
    TYPED[key]=set(m.group(1).rstrip() for m in re.finditer(r'=> b"([ -~]{4})"',blk))
TYPED['Brand']=TYPED['Brand']|TYPED.pop('ImageBrand')  # combined for KNOWN_BRANDS

# spec map -------------------------------------------------------------------------
HELD={'ISO','NALu Video','HEIF','3GPP','QT','DASH','AVIF','AV1-ISOBMFF','Dolby Vision','ETSI AC-3','ETSI AC-4','DTS','Event Message'}
NALU_POST_2017={'acgl','amgl','vvcb','evir','evc1','evcC','evs1','evs2','evsC','evm1','nnr1','nnrC','dcfi','lbli','minp','oinf','opth','pss1','rror','spid','spli','spor','sulm','trif','tsas','vopi','pprr','nfas','nfcs','nnet','mixn','recr','subp','vref','vreg','vvcN','supm','evcr','snut','cstg','alte','swtk'}
def held(code,spec):
    if spec=='NALu Video' and code in NALU_POST_2017: return False
    return spec in HELD

# table typed kinds ------------------------------------------------------------------
TYPED_KINDS={
 'aux-info-types':['AuxInfoType'],
 'boxes':['Box'],
 'boxes-qt':['QtAtom','Box'],
 'boxes-udta':['UdtaAtom'],
 'brands':['Brand'],
 'checksum-types':[],
 'color-types':['ColourType'],
 'data-references':['DataReference'],
 'entity-groups':['EntityGroup'],
 'handlers':['Handler'],
 'item-properties':['ItemProperty'],
 'item-references':['Reference'],
 'item-types':['ItemType'],
 'key-namespaces':['KeyNamespace'],
 'knownduplicates':[],
 'multiview-attributes':['TrackSelection','Box'],
 'oti':[],
 'sample-entries':['SampleEntry'],
 'sample-entries-boxes':['SampleEntryBox','ItemProperty','Box'],
 'sample-entries-qt':['SampleEntry'],
 'sample-groups':['SampleGroup'],
 'schemes':['Scheme'],
 'stream-types':[],
 'textualcontent':[],
 'track-groups':['TrackGroup'],
 'track-references':['TrackReference'],
 'track-references-qt':['TrackReference'],
 'track-selection':['TrackSelection'],
 'uncv-profiles':[],
 'unlisted':['Box','SampleEntry','SampleEntryBox','QtAtom','KeyNamespace'],
}
NA_ALL={'oti':'not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries',
 'stream-types':'not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries',
 'textualcontent':'not a four-character code: the table is a prose list of MIME/sentence checks',
 'uncv-profiles':'not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type',
 'knownduplicates':'the file is a one-line note naming a duplicate code, not a code table'}
NA_REASON={'checksum-types':'a digest algorithm identifier carried inside an integrity box, not a four-character box or entry type'}
NA_EXTRA={
 'boxes':{
  '!mof':'compressed-file container whose syntax the crate does not model (it is a box, pinned untyped-held instead)',
  'free':'free-space padding with no fields; preserved opaquely by the box walker',
  'skip':'skippable free-space padding with no fields; preserved opaquely by the box walker',
  'uuid':'extensible box header: the usertype selects the structure, so there is no single syntax',
  'fdel':'an item-information extension carried inside infe, not a box of its own',
  'clip':'reserved by the registry; no syntax is defined',
  'crgn':'reserved by the registry; no syntax is defined',
  'ctab':'reserved by the registry; no syntax is defined',
  'imap':'reserved by the registry; no syntax is defined',
  'kmat':'reserved by the registry; no syntax is defined',
  'matt':'reserved by the registry; no syntax is defined',
  'pnot':'reserved by the registry; no syntax is defined',
 },
 'boxes-qt':{
  'clip':'reserved by the registry; no syntax is defined',
  'crgn':'reserved by the registry; no syntax is defined',
  'ctab':'reserved by the registry; no syntax is defined',
  'imap':'reserved by the registry; no syntax is defined',
  'kmat':'reserved by the registry; no syntax is defined',
  'matt':'reserved by the registry; no syntax is defined',
  'pnot':'reserved by the registry; no syntax is defined',
 },
 'unlisted':{
  '?vc?':'a registry pattern for sample-entry codes, not a literal four-CC',
  '3gLZ':'a registry pattern for brand codes, not a literal four-CC',
  'abcd':'a registry example, not a real code',
  'encX':'a registry template for encrypted entries, not a literal four-CC',
  'qvXt':'a registry example brand, not a real code',
  'stuf':'a registry example metadata space, not a real code',
 },
}
NA_EXTRA['boxes'].pop('!mof')

all_codes=set()
for t in TABLES:
    for c,_ in rows[t]: all_codes.add(c)

def classify(t):
    typed=set()
    for k in TYPED_KINDS[t]:
        typed |= TYPED[k] & {c for c,_ in rows[t]}
    na=[]
    if t in NA_ALL:
        for c,_ in rows[t]: na.append((c,NA_ALL[t]))
    elif t in NA_REASON:
        for c,_ in rows[t]: na.append((c,NA_REASON[t]))
    for c,reason in NA_EXTRA.get(t,{}).items():
        if any(code==c for code,_ in rows[t]): na.append((c,reason))
    na_set={c for c,_ in na}
    heldc=[]; unheldc=[]
    for c,r in rows[t]:
        if c in typed or c in na_set: continue
        spec=(r.get('specification') or '').strip()
        (heldc if held(c,spec) else unheldc).append(c)
    return sorted(typed), sorted(heldc), sorted(unheldc), sorted(na)

out=[]
for t in TABLES:
    typed,heldc,unheldc,na=classify(t)
    out.append((t,len(rows[t]),typed,heldc,unheldc,na))


# emission -------------------------------------------------------------------------
KIND_TABLE={
 'Box':'TYPED_BOXES_MP4RA','SampleEntry':'TYPED_SAMPLE_ENTRIES','SampleEntryBox':'TYPED_SAMPLE_ENTRY_BOXES',
 'ItemProperty':'TYPED_ITEM_PROPERTIES','ItemType':'TYPED_ITEM_TYPES','Reference':'TYPED_REFERENCES',
 'EntityGroup':'TYPED_ENTITY_GROUPS','SampleGroup':'TYPED_SAMPLE_GROUPS','TrackGroup':'TYPED_TRACK_GROUPS',
 'TrackReference':'TYPED_TRACK_REFERENCES','TrackSelection':'TYPED_TRACK_SELECTION',
 'KeyNamespace':'TYPED_KEY_NAMESPACES','Scheme':'TYPED_SCHEMES','AuxInfoType':'TYPED_AUX_INFO_TYPES',
 'ColourType':'TYPED_COLOUR_TYPES','DataReference':'TYPED_DATA_REFERENCES','Brand':'KNOWN_BRANDS',
 'Handler':'KNOWN_HANDLERS','QtAtom':'TYPED_QT_ATOMS','UdtaAtom':'TYPED_UDTA_ATOMS',
}
KIND_ORDER=['Box','QtAtom','UdtaAtom','SampleEntry','SampleEntryBox','ItemProperty','ItemType','Reference','EntityGroup','SampleGroup','TrackGroup','TrackReference','TrackSelection','KeyNamespace','Scheme','AuxInfoType','ColourType','DataReference','Brand','Handler']

NOT_REG_REASONS={
 ('SampleEntry','MAC3'):'MACE 3:1 QuickTime sound entry; Apple writes it but no MP4RA table registers it',
 ('SampleEntry','MAC6'):'MACE 6:1 QuickTime sound entry; Apple writes it but no MP4RA table registers it',
 ('SampleEntry','NONE'):'QuickTime uncompressed sound entry; no MP4RA table registers it',
 ('SampleEntry','sowt'):'QuickTime little-endian PCM entry; common in .mov files but no MP4RA table registers it',
 ('SampleEntry','lav2'):'AVS2 library sample entry the crate types; MP4RA registers the avst entry only',
 ('SampleEntry','lav3'):'AVS3 library sample entry the crate types; MP4RA registers the avs3 entry only',
 ('SampleEntryBox','av2c'):'AVS2 configuration box the crate types; no MP4RA table registers it',
 ('SampleEntryBox','av3c'):'AVS3 configuration box the crate types; no MP4RA table registers it',
 ('SampleEntryBox','la2c'):'AVS2 library configuration box the crate types; no MP4RA table registers it',
 ('SampleEntryBox','lavc'):'AVS3 library configuration box the crate types; no MP4RA table registers it',
 ('SampleEntryBox','dOps'):'Opus decoder configuration box (RFC 7845); no MP4RA table registers it',
 ('SampleEntryBox','ddts'):'DTS decoder configuration box; MP4RA registers the DTS sample entries but not this box',
 ('ItemProperty','a1op'):'AVIF operating-point selector property the crate types; no MP4RA table registers it',
 ('ItemProperty','a1lx'):'AVIF layered-image-indexing property the crate types; no MP4RA table registers it',
 ('ItemProperty','cmex'):'ISO/IEC 23008-12:2025 camera extrinsic matrix property; not yet in MP4RA',
 ('ItemProperty','cmin'):'ISO/IEC 23008-12:2025 camera intrinsic matrix property; not yet in MP4RA',
 ('ItemProperty','prdi'):'ISO/IEC 23008-12:2025 progressive derived item information property; not yet in MP4RA',
 ('ItemProperty','sstr'):'ISO/IEC 23008-12:2025 single-stream property; not yet in MP4RA',
 ('Reference','anot'):'ISO/IEC 23008-12:2025 region annotation reference; not yet in MP4RA',
 ('Reference','drgn'):'ISO/IEC 23008-12:2025 derived-region reference; not yet in MP4RA',
 ('EntityGroup','prgr'):'progressive rendering entity group; MP4RA registers the slideshow and pyramid groups but not this one',
 ('SampleGroup','regm'):'ISO/IEC 23008-12:2025 region annotation sample group; not yet in MP4RA',
}

def arr(codes):
    if not codes: return '&[]'
    return '&[\n' + ''.join(f'    TypedRow {{ code: "{c}" }},\n' for c in codes) + ']'

def codes_of(t):
    return [c for c,_ in rows[t]]

def rust_str(s): return s.replace('\\','\\\\').replace('"','\\"')

def emit():
    L=[]
    L.append('''//! MP4RA registration coverage: every code in every table the MP4 Registration
//! Authority publishes, partitioned into typed, pinned untyped (with and
//! without a held specification) and not-applicable.
//!
//! The tables are generated from `private/specs-verified/mpeg/isobmff/mp4ra/`
//! and pinned here so `tests/it/mp4ra.rs` can check the partition against the
//! CSVs. A typed row proves itself: a structured code parses a minimal instance
//! through the crate's parser, an enumerated identifier maps through its typed
//! enum and back. A code in no list, or in two, fails the test.
//!
//! A specification counts as held only when it is under `private/specs/` or
//! transcribed under `private/specs-verified/`. A code whose defining document
//! is not held is pinned as such rather than implemented from memory.
//!
//! Regenerate with `tools/gen-mp4ra-coverage.py` after editing the typed sets
//! or adding a held document.

/// One code a typed row names.
#[derive(Debug, Clone, Copy)]
pub struct TypedRow {
    /// The four-CC.
    pub code: &'static str,
}

/// A code that is not a structure this crate parses, with the reason.
#[derive(Debug, Clone, Copy)]
pub struct NotApplicable {
    /// The four-CC or table value.
    pub code: &'static str,
    /// Why the code is not an ISOBMFF structure.
    pub reason: &'static str,
}

/// The kind of identifier a typed table names, which selects its proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedKind {
    /// A registered box.
    Box,
    /// A QuickTime atom outside the ISO box set.
    QtAtom,
    /// A QuickTime/3GPP user-data atom.
    UdtaAtom,
    /// A sample entry.
    SampleEntry,
    /// A configuration or extension box inside a sample entry.
    SampleEntryBox,
    /// An item property.
    ItemProperty,
    /// An item type.
    ItemType,
    /// An item reference type.
    Reference,
    /// An entity-grouping type.
    EntityGroup,
    /// A sample-grouping type.
    SampleGroup,
    /// A track-grouping type.
    TrackGroup,
    /// A track-reference type.
    TrackReference,
    /// A track-selection attribute.
    TrackSelection,
    /// A metadata key namespace.
    KeyNamespace,
    /// A restricted or protection scheme type.
    Scheme,
    /// A sample-auxiliary-information type.
    AuxInfoType,
    /// A colour type.
    ColourType,
    /// A data-reference entry type.
    DataReference,
    /// A file brand.
    Brand,
    /// A handler type.
    Handler,
}

impl TypedKind {
    /// The typed table this kind reads.
    #[must_use]
    pub fn rows(self) -> &'static [TypedRow] {
        match self {''')
    for k in KIND_ORDER:
        L.append(f'            Self::{k} => {KIND_TABLE[k]},')
    L.append('''        }
    }
}
''')
    for k in KIND_ORDER:
        codes=sorted(TYPED[k])
        L.append(f'\n/// The {k} codes a held document defines and this crate types.\npub const {KIND_TABLE[k]}: &[TypedRow] = {arr(codes)};\n')
    # typed not registered
    L.append('\n/// A typed code no MP4RA table lists, with the explanation the README states.\n#[derive(Debug, Clone, Copy)]\npub struct TypedNotRegistered {\n    /// The kind of identifier.\n    pub kind: TypedKind,\n    /// The four-CC.\n    pub code: &\'static str,\n    /// Why the crate types it anyway.\n    pub reason: &\'static str,\n}\n\n/// Typed codes no MP4RA table registers, each explained.\npub const TYPED_NOT_REGISTERED: &[TypedNotRegistered] = &[\n')
    for k in KIND_ORDER:
        for c in sorted(TYPED[k]-all_codes):
            reason=NOT_REG_REASONS.get((k,c),'typed by the crate but absent from every MP4RA table')
            L.append(f'    TypedNotRegistered {{ kind: TypedKind::{k}, code: "{c}", reason: "{rust_str(reason)}" }},\n')
    L.append('];\n\n')
    # tables
    L.append('/// One MP4RA table and its partition.\n#[derive(Debug)]\npub struct Table {\n    /// The CSV file stem under `private/specs-verified/mpeg/isobmff/mp4ra/`.\n    pub csv: &\'static str,\n    /// The number of distinct codes in the CSV (duplicate rows are one code).\n    pub rows: usize,\n    /// The typed tables whose codes this table reads.\n    pub typed: &\'static [TypedKind],\n    /// Codes the crate does not type, whose defining document is held.\n    pub untyped_held: &\'static [&\'static str],\n    /// Codes the crate does not type, whose defining document is not held.\n    pub untyped_unheld: &\'static [&\'static str],\n    /// Codes that are not ISOBMFF structures this crate parses.\n    pub not_applicable: &\'static [NotApplicable],\n}\n\n/// Every MP4RA table, in the order the private README lists them.\npub const TABLES: &[Table] = &[\n')
    for t,n,typed,h,u,na in out:
        kinds=TYPED_KINDS[t]
        kindlist='&[' + ', '.join(f'TypedKind::{k}' for k in kinds) + ']'
        L.append(f'    Table {{\n        csv: "{t}",\n        rows: {n},\n        typed: {kindlist},\n        untyped_held: &[{", ".join(chr(34)+c+chr(34) for c in h)}],\n        untyped_unheld: &[{", ".join(chr(34)+c+chr(34) for c in u)}],\n        not_applicable: &[{", ".join(f"NotApplicable {{ code: {chr(34)}{c}{chr(34)}, reason: {chr(34)}{rust_str(r)}{chr(34)} }}" for c,r in na)}],\n    }},\n')
    L.append('];\n')
    os.makedirs(ROOT/'isobmff/src/coverage',exist_ok=True)
    open(ROOT/'isobmff/src/coverage/mp4ra.rs','w').write(''.join(L))


