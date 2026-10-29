//! Per-meeting snapshot folders: `<root>\<yyyy-MM-dd HH.mm> <title>\HH.mm.ss.png`, the Mac's
//! `SnapshotStorage.swift` with Windows-safe names.
//!
//! Core's retention deletes a folder only if it holds `api::retention::SNAPSHOT_MARKER`
//! and otherwise only PNGs named as [`file_name`] makes them (`retention::is_snapshot_png`),
//! so nothing else is written here, and the two must stay in step.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{NaiveDateTime, NaiveTime};

use crate::capture::frame::Frame;

/// The default root's name under the user's Pictures folder, as on the Mac.
pub const ROOT_NAME: &str = "Whimbrel Snapshots";
/// The longest title kept in a folder name, in characters.
const TITLE_CHARS: usize = 80;

/// The default root: `Pictures\Whimbrel Snapshots`. Pictures comes from the known-folder API,
/// since it may be redirected (to OneDrive, say). `None` if Windows can't say where it is.
pub fn default_root() -> Option<PathBuf> {
    use windows::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    let pictures = unsafe { SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None).ok()? };
    let path = unsafe { pictures.to_string().ok() };
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(pictures.0 as *const _)) };
    Some(PathBuf::from(path?).join(ROOT_NAME))
}

/// Where new meetings' folders go: the folder chosen in Settings, else the default root,
/// else `fallback` (under the data folder).
pub fn effective_root(chosen: Option<PathBuf>, fallback: &Path) -> PathBuf {
    chosen.or_else(default_root).unwrap_or_else(|| fallback.to_path_buf())
}

/// `"yyyy-MM-dd HH.mm <title>"`, with `start` in local time and the title made safe.
pub fn folder_name(start: NaiveDateTime, title: &str) -> String {
    format!("{} {}", start.format("%Y-%m-%d %H.%M"), safe_title(title))
}

/// A title Windows accepts as a folder name: reserved and control characters become `-`,
/// trailing dots and spaces go (Explorer can't open such a folder), reserved device names
/// get a suffix, and it's cut to `TITLE_CHARS`. Empty becomes `Meeting`.
fn safe_title(title: &str) -> String {
    let replaced: String = title.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '-' } else { c }).collect();
    let trim = |s: &str| s.trim().trim_start_matches('.').trim_end_matches(['.', ' ']).trim().to_string();
    let mut t = trim(&replaced);
    t = trim(&t.chars().take(TITLE_CHARS).collect::<String>());
    if t.is_empty() {
        return "Meeting".into();
    }
    // `CON`, `NUL.txt` and the like name devices, not files. The date prefix already keeps the
    // whole folder name clear of them; this covers a caller that uses the title alone.
    let stem = t.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit() && stem.as_bytes()[3] != b'0');
    if reserved {
        t.push_str(" meeting");
    }
    t
}

/// Makes the meeting's folder under `root`, or under `fallback` if `root` can't be written.
/// A name already taken gets ` 2`, ` 3` and so on. The folder gets the marker file.
pub fn make_meeting_folder(root: &Path, fallback: &Path, start: NaiveDateTime, title: &str) -> io::Result<PathBuf> {
    let name = folder_name(start, title);
    create(&name, root).or_else(|_| create(&name, fallback))
}

fn create(name: &str, parent: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(parent)?;
    for n in 1.. {
        let folder = parent.join(if n == 1 { name.to_string() } else { format!("{name} {n}") });
        match std::fs::create_dir(&folder) {
            Ok(()) => {
                std::fs::write(folder.join(api::retention::SNAPSHOT_MARKER), b"")?;
                return Ok(folder);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("1.. never ends")
}

/// `HH.mm.ss.png`, or `HH.mm.ss-2.png` and so on if that's taken.
pub fn file_name(captured: NaiveTime, existing: &HashSet<String>) -> String {
    let base = captured.format("%H.%M.%S").to_string();
    let mut name = format!("{base}.png");
    let mut n = 2;
    while existing.contains(&name) {
        name = format!("{base}-{n}.png");
        n += 1;
    }
    name
}

/// Writes `frame` as a PNG in `folder`, named for `captured` (local time), and returns its path.
pub fn write_png(frame: &Frame, folder: &Path, captured: NaiveTime) -> io::Result<PathBuf> {
    let existing: HashSet<String> =
        std::fs::read_dir(folder)?.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    let path = folder.join(file_name(captured, &existing));
    let file = std::fs::File::create_new(&path)?;
    let mut encoder = png::Encoder::new(io::BufWriter::new(file), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let rgb: Vec<u8> = frame.bgra.as_chunks::<4>().0.iter().flat_map(|[b, g, r, _]| [*r, *g, *b]).collect();
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(&rgb).map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("whimbrel-snapstore-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_folder_name_is_the_local_start_then_the_title() {
        assert_eq!(folder_name(at("2026-10-22 10:14:59"), "Acme sync"), "2026-10-22 10.14 Acme sync");
    }

    #[test]
    fn hostile_titles_become_safe_folder_names() {
        let name = |t: &str| folder_name(at("2026-10-22 10:14:00"), t)["2026-10-22 10.14 ".len()..].to_string();
        assert_eq!(name(r#"a/b:c\d<e>f"g|h?i*j"#), "a-b-c-d-e-f-g-h-i-j");
        assert_eq!(name("..hidden"), "hidden");
        assert_eq!(name("Q3 review. . "), "Q3 review");
        assert_eq!(name("tab\there"), "tab-here");
        assert_eq!(name("   "), "Meeting");
        assert_eq!(name("🚀 Launch"), "🚀 Launch");
        assert_eq!(name("CON"), "CON meeting");
        assert_eq!(name("com1.notes"), "com1.notes meeting");
        assert_eq!(name("COM0"), "COM0", "COM0 isn't reserved");
        assert_eq!(name("Console"), "Console");
        assert!(name(&"x".repeat(300)).chars().count() <= TITLE_CHARS);
    }

    #[test]
    fn a_second_meeting_with_the_same_name_gets_a_suffix() {
        let root = temp("collide");
        let start = at("2026-10-22 10:14:00");
        let a = make_meeting_folder(&root, &root, start, "Acme sync").unwrap();
        let b = make_meeting_folder(&root, &root, start, "Acme sync").unwrap();
        assert_eq!(a.file_name().unwrap(), "2026-10-22 10.14 Acme sync");
        assert_eq!(b.file_name().unwrap(), "2026-10-22 10.14 Acme sync 2");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_new_folder_holds_the_marker_core_looks_for() {
        let root = temp("marker");
        let folder = make_meeting_folder(&root, &root, at("2026-10-22 10:14:00"), "Acme").unwrap();
        assert!(folder.join(api::retention::SNAPSHOT_MARKER).is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_unwritable_root_falls_back() {
        let base = temp("fallback");
        let not_a_dir = base.join("root-is-a-file");
        std::fs::write(&not_a_dir, b"x").unwrap();
        let folder = make_meeting_folder(&not_a_dir, &base.join("fallback"), at("2026-10-22 10:14:00"), "Acme").unwrap();
        assert!(folder.starts_with(base.join("fallback")), "{}", folder.display());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_file_name_taken_in_the_same_second_gets_a_suffix() {
        let t = NaiveTime::from_hms_opt(10, 14, 3).unwrap();
        assert_eq!(file_name(t, &HashSet::new()), "10.14.03.png");
        let taken: HashSet<String> = ["10.14.03.png".to_string(), "10.14.03-2.png".to_string()].into();
        assert_eq!(file_name(t, &taken), "10.14.03-3.png");
    }

    #[test]
    fn a_written_png_round_trips_its_pixels_and_core_counts_the_folder_as_its_own() {
        let root = temp("png");
        let folder = make_meeting_folder(&root, &root, at("2026-10-22 10:14:00"), "Acme").unwrap();
        let bgra = (0..6 * 4).flat_map(|i| [i as u8, 100, 200, 255]).collect();
        let frame = Frame { bgra, width: 6, height: 4 };
        let path = write_png(&frame, &folder, NaiveTime::from_hms_opt(10, 14, 3).unwrap()).unwrap();
        assert_eq!(path.file_name().unwrap(), "10.14.03.png");

        let decoder = png::Decoder::new(io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (6, 4));
        assert_eq!(&buf[0..6], &[200, 100, 0, 200, 100, 1], "RGB, from BGRA");

        // What `write_png` leaves is exactly what retention may delete.
        assert!(api::retention::delete_snapshots_dir(&folder), "a marked folder of our PNGs is ours");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_default_root_is_under_pictures() {
        let root = default_root().expect("a Pictures folder");
        assert_eq!(root.file_name().unwrap(), ROOT_NAME);
        assert!(root.parent().unwrap().is_dir(), "{}", root.display());
    }
}
