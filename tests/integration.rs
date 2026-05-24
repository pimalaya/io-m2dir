//! End-to-end m2dir test flow.
//!
//! Drives the full [`M2dirClient`] surface against a freshly created
//! tempdir. The test is autonomous: it provisions its own m2store, its
//! own m2dirs, and its own messages, then exercises every public
//! operation in sequence:
//!
//! ```text
//! INIT STORE
//!   → MAILBOX CREATE inbox / sent / nested
//!   → MAILBOX LIST            (verify all three are visible)
//!   → OPEN M2DIR inbox
//!   → MESSAGE STORE x3        (inbox)
//!   → MESSAGE LIST            (verify count + ids match)
//!   → MESSAGE GET             (verify checksum + body round-trip)
//!   → FLAGS ADD $seen, $forwarded
//!   → FLAGS READ              (verify both present)
//!   → FLAGS REMOVE $seen
//!   → FLAGS READ              (verify only $forwarded remains)
//!   → FLAGS SET custom, $junk
//!   → FLAGS READ              (verify replacement)
//!   → FLAGS SET <empty>       (verify .flags file is removed)
//!   → MESSAGE DELETE          (verify file + meta gone)
//!   → MAILBOX DELETE sent
//!   → MAILBOX LIST            (verify sent gone, two remain)
//! ```

use std::path::Path;

use io_m2dir::{client::M2dirClient, flag::M2dirFlags, m2dir::DOT_M2DIR, m2store::DOT_M2STORE};
use tempfile::tempdir;

#[test]
fn end_to_end() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path().to_string_lossy().into_owned();
    let client = M2dirClient::new(root);

    // ── INIT STORE ──────────────────────────────────────────────────

    let store = client.init_store().expect("init m2store");
    assert!(
        Path::new(store.marker_path().as_str()).exists(),
        "store marker {} should exist",
        DOT_M2STORE,
    );

    // ── MAILBOX CREATE ──────────────────────────────────────────────

    let inbox = client.create_mailbox("inbox").expect("create inbox");
    let sent = client.create_mailbox("sent").expect("create sent");
    let nested = client
        .create_mailbox("archives/2026")
        .expect("create nested mailbox");

    for m2dir in [&inbox, &sent, &nested] {
        assert!(
            Path::new(m2dir.path().as_str()).is_dir(),
            "{} should be a directory",
            m2dir.path(),
        );
        assert!(
            Path::new(m2dir.marker_path().as_str()).exists(),
            "{} marker should exist in {}",
            DOT_M2DIR,
            m2dir.path(),
        );
        assert!(
            Path::new(m2dir.meta_dir().as_str()).is_dir(),
            ".meta dir should exist in {}",
            m2dir.path(),
        );
    }

    // ── MAILBOX LIST (baseline) ─────────────────────────────────────

    let mailboxes = client.list_mailboxes().expect("list mailboxes");
    assert_eq!(mailboxes.len(), 3, "expected three mailboxes after create");
    assert!(mailboxes.contains(&inbox), "inbox missing from listing");
    assert!(mailboxes.contains(&sent), "sent missing from listing");
    assert!(mailboxes.contains(&nested), "nested missing from listing");

    // ── OPEN M2DIR (round-trip path → handle) ───────────────────────

    let reopened = client
        .open_m2dir(inbox.path().clone())
        .expect("re-open inbox by path");
    assert_eq!(
        reopened.path(),
        inbox.path(),
        "re-opened m2dir path mismatch",
    );

    // ── MESSAGE STORE x3 ────────────────────────────────────────────

    let body_a = build_eml("alice@example.org", "first");
    let body_b = build_eml("bob@example.org", "second");
    let body_c = build_eml("carol@example.org", "third");

    let entry_a = client
        .store(inbox.clone(), body_a.clone().into_bytes())
        .expect("store first message");
    let entry_b = client
        .store(inbox.clone(), body_b.clone().into_bytes())
        .expect("store second message");
    let entry_c = client
        .store(inbox.clone(), body_c.clone().into_bytes())
        .expect("store third message");

    for entry in [&entry_a, &entry_b, &entry_c] {
        assert!(
            Path::new(entry.path().as_str()).is_file(),
            "{} should be a regular file",
            entry.path(),
        );
    }
    assert_ne!(entry_a.id(), entry_b.id(), "ids should be unique");
    assert_ne!(entry_b.id(), entry_c.id(), "ids should be unique");
    assert_ne!(entry_a.id(), entry_c.id(), "ids should be unique");

    // ── MESSAGE LIST ────────────────────────────────────────────────

    let listed = client.list_entries(inbox.clone()).expect("list messages");
    assert_eq!(listed.len(), 3, "expected three messages after store");
    let listed_ids: Vec<&str> = listed.iter().map(|e| e.id()).collect();
    assert!(listed_ids.contains(&entry_a.id()), "entry_a missing");
    assert!(listed_ids.contains(&entry_b.id()), "entry_b missing");
    assert!(listed_ids.contains(&entry_c.id()), "entry_c missing");

    // ── MESSAGE GET (checksum + body round-trip) ────────────────────

    let (fetched, contents) = client
        .get(inbox.clone(), entry_a.id())
        .expect("get first message");
    assert_eq!(fetched.id(), entry_a.id(), "fetched id mismatch");
    assert_eq!(contents, body_a.as_bytes(), "fetched body mismatch");

    // ── FLAGS ADD ───────────────────────────────────────────────────

    let initial = client
        .read_flags(&inbox, entry_a.id())
        .expect("initial flags");
    assert!(initial.is_empty(), "flags should start empty");

    let mut to_add = M2dirFlags::default();
    to_add.insert("$seen");
    to_add.insert("$forwarded");
    client
        .add_flags(&inbox, entry_a.id(), to_add)
        .expect("add flags");

    let after_add = client
        .read_flags(&inbox, entry_a.id())
        .expect("flags after add");
    assert_eq!(after_add.len(), 2, "expected 2 flags after add");
    assert!(after_add.contains("$seen"));
    assert!(after_add.contains("$forwarded"));

    // ── FLAGS REMOVE ────────────────────────────────────────────────

    let mut to_remove = M2dirFlags::default();
    to_remove.insert("$seen");
    client
        .remove_flags(&inbox, entry_a.id(), to_remove)
        .expect("remove flags");

    let after_remove = client
        .read_flags(&inbox, entry_a.id())
        .expect("flags after remove");
    assert_eq!(after_remove.len(), 1, "expected 1 flag after remove");
    assert!(after_remove.contains("$forwarded"));
    assert!(!after_remove.contains("$seen"));

    // ── FLAGS SET (replacement) ─────────────────────────────────────

    let mut replacement = M2dirFlags::default();
    replacement.insert("custom");
    replacement.insert("$junk");
    client
        .set_flags(&inbox, entry_a.id(), replacement)
        .expect("set flags");

    let after_set = client
        .read_flags(&inbox, entry_a.id())
        .expect("flags after set");
    assert_eq!(after_set.len(), 2, "expected 2 flags after set");
    assert!(after_set.contains("custom"));
    assert!(after_set.contains("$junk"));
    assert!(!after_set.contains("$forwarded"));

    // ── FLAGS SET <empty> (removes .flags file) ─────────────────────

    client
        .set_flags(&inbox, entry_a.id(), M2dirFlags::default())
        .expect("clear flags");

    let after_clear = client
        .read_flags(&inbox, entry_a.id())
        .expect("flags after clear");
    assert!(after_clear.is_empty(), "flags should be empty after clear");
    assert!(
        !Path::new(inbox.flags_path(entry_a.id()).as_str()).exists(),
        ".flags file should be removed when set to empty",
    );

    // ── MESSAGE DELETE ──────────────────────────────────────────────

    // Re-add a flag so we can confirm delete also wipes .meta entries.
    let mut flags = M2dirFlags::default();
    flags.insert("$seen");
    client
        .add_flags(&inbox, entry_b.id(), flags)
        .expect("add flag on entry_b");
    assert!(Path::new(inbox.flags_path(entry_b.id()).as_str()).exists());

    client
        .delete_message(inbox.clone(), entry_b.id())
        .expect("delete entry_b");
    assert!(
        !Path::new(entry_b.path().as_str()).exists(),
        "entry_b file should be gone",
    );
    assert!(
        !Path::new(inbox.flags_path(entry_b.id()).as_str()).exists(),
        "entry_b .flags should be gone",
    );

    let remaining = client
        .list_entries(inbox.clone())
        .expect("list after delete");
    assert_eq!(remaining.len(), 2, "expected 2 messages after delete");

    // ── MAILBOX DELETE ──────────────────────────────────────────────

    let sent_path = sent.path().clone();
    client
        .delete_mailbox(sent_path.clone())
        .expect("delete sent");
    assert!(
        !Path::new(sent_path.as_str()).exists(),
        "sent dir should be removed",
    );

    let mailboxes = client.list_mailboxes().expect("list after mailbox delete");
    assert_eq!(mailboxes.len(), 2, "expected 2 mailboxes after delete");
    assert!(mailboxes.contains(&inbox), "inbox still present");
    assert!(mailboxes.contains(&nested), "nested still present");
    assert!(!mailboxes.contains(&sent), "sent should be gone");
}

fn build_eml(from: &str, tag: &str) -> String {
    [
        &format!("From: io-m2dir test <{from}>"),
        &format!("To: io-m2dir test <{from}>"),
        &format!("Subject: io-m2dir integration test {tag}"),
        "Date: Thu, 01 Jan 2026 00:00:00 +0000",
        "MIME-Version: 1.0",
        "Content-Type: text/plain; charset=utf-8",
        "",
        &format!("This is automated test email {tag} from io-m2dir tests."),
    ]
    .join("\r\n")
}
