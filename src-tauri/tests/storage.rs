use gloss_desktop::storage::Storage;

#[test]
fn storage_survives_reopen_without_losing_original_pdf() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gloss.sqlite3");
    let id = "论文/../雪 '); DROP TABLE documents; -- 🦀";
    let document = r#"{"title":"雪と光","text":"it's a \"paper\""}"#;
    let bytes = vec![0, 255, 128, 37, 80, 68, 70, 10, 0];
    {
        let storage = Storage::open(&path).unwrap();
        assert_eq!(storage.load_state("missing").unwrap(), None);
        assert_eq!(storage.load_document("missing").unwrap(), None);
        assert_eq!(storage.load_pdf("missing").unwrap(), None);
        storage.save_state("gloss:v1", "旧").unwrap();
        storage.save_state("gloss:v1", "新 🦀").unwrap();
        storage
            .save_state("gloss:settings", r#"{"theme":"night"}"#)
            .unwrap();
        storage.save_pdf(id, &bytes).unwrap();
        assert_eq!(storage.load_document(id).unwrap(), None);
        storage.save_document(id, "old document").unwrap();
        storage.save_document(id, document).unwrap();
    }
    {
        let storage = Storage::open(&path).unwrap();
        assert_eq!(
            storage.load_state("gloss:v1").unwrap().as_deref(),
            Some("新 🦀")
        );
        assert_eq!(
            storage.load_state("gloss:settings").unwrap().as_deref(),
            Some(r#"{"theme":"night"}"#)
        );
        assert_eq!(
            storage.load_document(id).unwrap().as_deref(),
            Some(document)
        );
        assert_eq!(storage.load_pdf(id).unwrap(), Some(bytes.clone()));
        storage.forget_document(id).unwrap();
        assert_eq!(storage.load_document(id).unwrap(), None);
        assert_eq!(storage.load_pdf(id).unwrap(), Some(bytes.clone()));
    }
    {
        let storage = Storage::open(&path).unwrap();
        assert_eq!(storage.load_document(id).unwrap(), None);
        assert_eq!(storage.load_pdf(id).unwrap(), Some(bytes));
        storage.save_document(id, document).unwrap();
        storage.delete_document(id).unwrap();
        storage.delete_document("missing").unwrap();
        storage.forget_document("missing").unwrap();
    }
    let storage = Storage::open(&path).unwrap();
    assert_eq!(storage.load_document(id).unwrap(), None);
    assert_eq!(storage.load_pdf(id).unwrap(), None);
    assert!(storage.load_state("gloss:v1").unwrap().is_some());
}

#[test]
fn pdf_update_preserves_document_and_accepts_empty_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::open(&directory.path().join("test.sqlite3")).unwrap();
    storage.save_document("id", "body").unwrap();
    storage.save_pdf("id", &[1, 2]).unwrap();
    storage.save_pdf("id", &[]).unwrap();
    assert_eq!(
        storage.load_document("id").unwrap().as_deref(),
        Some("body")
    );
    assert_eq!(storage.load_pdf("id").unwrap(), Some(vec![]));
}
