use super::*;
use std::collections::HashSet;

fn taken(names: &[&str]) -> impl Fn(&str) -> bool {
    let set: HashSet<String> = names.iter().map(|s| s.to_string()).collect();
    move |n| set.contains(n)
}

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("swd-{name}-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn free_name_numbers_collisions() {
    assert_eq!(free_name("file.pdf", taken(&[])), "file.pdf");
    assert_eq!(free_name("file.pdf", taken(&["file.pdf"])), "file (1).pdf");
    assert_eq!(free_name("file.pdf", taken(&["file.pdf", "file (1).pdf"])), "file (2).pdf");
    assert_eq!(free_name("file.pdf", taken(&["file.pdf", "file (2).pdf"])), "file (1).pdf", "first gap");
    assert_eq!(free_name("README", taken(&["README"])), "README (1)");
    assert_eq!(free_name("a.tar.gz", taken(&["a.tar.gz"])), "a.tar (1).gz");
    assert_eq!(free_name(".bashrc", taken(&[".bashrc"])), ".bashrc (1)");
}

#[test]
fn sanitize_keeps_only_a_safe_last_component() {
    assert_eq!(sanitize_file_name("a/b\\c.txt"), "c.txt");
    assert_eq!(sanitize_file_name("../../evil.exe"), "evil.exe");
    assert_eq!(sanitize_file_name("we<ir>d:na\"me|?*.txt"), "we_ir_d_na_me___.txt");
    assert_eq!(sanitize_file_name("tab\there.txt"), "tab_here.txt");
    assert_eq!(sanitize_file_name("name. . "), "name");
    for bad in ["", ".", "..", "/", "...", "  "] {
        assert_eq!(sanitize_file_name(bad), "download", "{bad:?}");
    }
    assert_eq!(sanitize_file_name("CON.txt"), "_CON.txt");
    assert_eq!(sanitize_file_name("nul"), "_nul");
    assert_eq!(sanitize_file_name("console.txt"), "console.txt");
    assert_eq!(sanitize_file_name("Ação 日本.pdf"), "Ação 日本.pdf");
}

#[test]
fn a_long_name_is_cut_but_keeps_its_extension() {
    let long = format!("{}.pdf", "x".repeat(400));
    let cut = sanitize_file_name(&long);
    assert_eq!(cut.chars().count(), 150);
    assert!(cut.ends_with(".pdf"));
}

#[test]
fn free_path_looks_on_disk() {
    let dir = tmp("free");
    std::fs::write(dir.join("a.txt"), b"x").unwrap();
    assert_eq!(free_path(&dir, "a.txt"), dir.join("a (1).txt"));
    assert_eq!(free_path(&dir, "b.txt"), dir.join("b.txt"));
}

#[test]
fn a_download_runs_to_the_end() {
    let mut d = Downloads::new();
    let id = d.begin("https://user:pw@files.example.com/x/f.zip?sig=1", "C:\\Users\\me\\Downloads\\f.zip", 0, true, 1000);
    let e = d.get(id).unwrap();
    assert_eq!((e.name.as_str(), e.from.as_str(), e.state), ("f.zip", "files.example.com", State::InProgress));
    assert_eq!(e.url, "https://files.example.com/x/f.zip?sig=1", "no password in what is kept");
    assert_eq!(e.fraction(), None, "size unknown");
    assert!(d.progress(id, 50, 200));
    assert!(!d.progress(id, 50, 200), "no change, no redraw");
    assert!(d.progress(id, 100, 0), "total 0 does not erase a known one");
    assert_eq!(d.get(id).unwrap().fraction(), Some(0.5));
    assert_eq!(d.active(), 1);
    assert!(d.finish(id));
    let e = d.get(id).unwrap();
    assert_eq!((e.state, e.received, e.total), (State::Done, 200, 200));
    assert_eq!(d.active(), 0);
    assert!(!d.progress(id, 1, 1), "an ended download takes no more progress");
    assert!(!d.fail(id, "late"));
}

#[test]
fn pause_resume_cancel_intents() {
    let mut d = Downloads::new();
    let id = d.begin("https://a.com/f", "C:\\d\\f", 10, true, 0);
    assert!(d.allows(id, Intent::Pause) && !d.allows(id, Intent::Resume) && d.allows(id, Intent::Cancel));
    assert!(d.set_paused(id, true));
    assert!(!d.allows(id, Intent::Pause) && d.allows(id, Intent::Resume) && d.allows(id, Intent::Cancel));
    assert_eq!(d.active(), 1, "paused is still under way");
    assert!(d.set_paused(id, false));
    assert!(d.cancel(id));
    assert_eq!(d.get(id).unwrap().state, State::Cancelled);
    assert!(!d.allows(id, Intent::Cancel) && !d.cancel(id) && !d.set_paused(id, true));
    assert!(!d.allows(999, Intent::Pause));
}

#[test]
fn failure_keeps_the_reason() {
    let mut d = Downloads::new();
    let id = d.begin("https://a.com/f", "C:\\d\\f", 0, true, 0);
    assert!(d.fail(id, "Network error"));
    let e = d.get(id).unwrap();
    assert_eq!((e.state, e.error.as_deref()), (State::Failed, Some("Network error")));
}

#[test]
fn clear_finished_leaves_what_is_under_way() {
    let mut d = Downloads::new();
    let (a, b, c, run) = (
        d.begin("https://a.com/1", "C:\\d\\1", 0, true, 1),
        d.begin("https://a.com/2", "C:\\d\\2", 0, true, 2),
        d.begin("https://a.com/3", "C:\\d\\3", 0, true, 3),
        d.begin("https://a.com/4", "C:\\d\\4", 0, true, 4),
    );
    d.finish(a);
    d.fail(b, "x");
    d.cancel(c);
    assert_eq!(d.clear_finished(), 3);
    assert_eq!(d.entries().len(), 1);
    assert_eq!(d.entries()[0].id, run);
    assert!(!d.forget(run), "under way: cancel it first");
    d.cancel(run);
    assert!(d.forget(run) && d.entries().is_empty());
}

#[test]
fn newest_first_and_same_path_replaces() {
    let mut d = Downloads::new();
    let a = d.begin("https://a.com/f", "C:\\d\\f.pdf", 0, true, 1);
    d.finish(a);
    let b = d.begin("https://b.com/f", "C:\\d\\g.pdf", 0, true, 2);
    d.finish(b);
    assert_eq!(d.entries().iter().map(|e| e.id).collect::<Vec<_>>(), vec![b, a]);
    let again = d.begin("https://c.com/f", "C:\\d\\f.pdf", 0, true, 3);
    d.finish(again);
    assert_eq!(d.entries().iter().map(|e| e.id).collect::<Vec<_>>(), vec![again, b]);
}

#[test]
fn only_fifty_finished_are_kept() {
    let mut d = Downloads::new();
    for i in 0..60 {
        let id = d.begin("https://a.com/f", &format!("C:\\d\\{i}"), 0, true, i);
        d.finish(id);
    }
    let run = d.begin("https://a.com/slow", "C:\\d\\slow", 0, true, 99);
    assert_eq!(d.entries().len(), 51);
    assert!(d.get(run).is_some());
    assert_eq!(d.entries().last().unwrap().path, "C:\\d\\10", "the oldest went");
}

#[test]
fn private_and_unfinished_are_not_written() {
    let dir = tmp("save");
    let path = dir.join("downloads.json");
    let mut d = Downloads::new();
    let done = d.begin("https://a.com/f", "C:\\d\\f", 0, true, 1);
    d.finish(done);
    let private = d.begin("https://a.com/p", "C:\\d\\p", 0, false, 2);
    d.finish(private);
    d.begin("https://a.com/run", "C:\\d\\run", 0, true, 3);
    let failed = d.begin("https://a.com/bad", "C:\\d\\bad", 0, true, 4);
    d.fail(failed, "x");
    d.save(&path).unwrap();
    assert!(!dir.join("downloads.json.tmp").exists());
    let back = Downloads::load(&path);
    assert_eq!(back.entries().len(), 1);
    assert_eq!(back.entries()[0].path, "C:\\d\\f");
    assert_eq!(back.entries()[0].state, State::Done);
    let mut back = back;
    let next = back.begin("https://a.com/n", "C:\\d\\n", 0, true, 5);
    assert!(next > done, "ids go on after the file's");
}

#[test]
fn missing_corrupt_or_edited_file_is_safe() {
    let dir = tmp("bad");
    let path = dir.join("downloads.json");
    assert!(Downloads::load(&path).entries().is_empty());
    std::fs::write(&path, b"[{ nope").unwrap();
    assert!(Downloads::load(&path).entries().is_empty());
    std::fs::write(
        &path,
        br#"[{"id":7,"state":"inprogress","path":"C:\\x"},{"id":8,"state":"done","path":"C:\\y","name":"y"}]"#,
    )
    .unwrap();
    let d = Downloads::load(&path);
    assert_eq!(d.entries().len(), 1, "nothing is under way after a restart");
    assert_eq!(d.entries()[0].id, 8);
}

#[test]
fn history_cleared_forgets_addresses_only() {
    let mut d = Downloads::new();
    let id = d.begin("https://a.com/f", "C:\\d\\f", 0, true, 1);
    d.finish(id);
    assert!(d.forget_addresses());
    assert_eq!(d.get(id).unwrap().url, "");
    assert_eq!(d.get(id).unwrap().from, "a.com");
    assert!(!d.forget_addresses());
}

#[test]
fn only_web_addresses_are_kept() {
    assert_eq!(address_of("blob:https://a.com/uuid"), "");
    assert_eq!(address_of("data:text/plain,hi"), "");
    assert_eq!(address_of("https://a.com/x"), "https://a.com/x");
}

#[test]
fn set_path_renames_the_row() {
    let mut d = Downloads::new();
    let id = d.begin("https://a.com/f", "C:\\d\\f.tmp", 0, true, 1);
    assert!(d.set_path(id, "C:\\d\\f.pdf"));
    assert_eq!(d.get(id).unwrap().name, "f.pdf");
}
