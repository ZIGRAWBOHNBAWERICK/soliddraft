//! How a shell's openings read and write in a part file. `[]` is a closed shell and `[{}]`
//! opens every face; both stay apart from picked faces, and older files read unchanged.

use serde_json::{Value, json};
use soliddraft_model::{ShellFeature, ShellOpenings};

fn picked() -> Value {
    json!({"feature": 3, "generator": {"kind": "end_cap"}, "fingerprint":
        {"surface": "plane", "normal": [0.0, 0.0, 1.0], "centroid": [0.0, 0.0, 12.0], "area": 1200.0}})
}

fn read(openings: Value) -> Result<ShellFeature, serde_json::Error> {
    serde_json::from_value(json!({"open_faces": openings, "thickness": 2.0}))
}

#[test]
fn no_faces_and_every_face_round_trip_as_different_lists() {
    let closed = read(json!([])).unwrap();
    let every = read(json!([{}])).unwrap();
    assert_eq!(closed.open_faces, ShellOpenings::Faces(Vec::new()));
    assert_eq!(every.open_faces, ShellOpenings::All);
    assert_ne!(closed, every);
    assert_eq!(serde_json::to_value(&closed).unwrap()["open_faces"], json!([]));
    assert_eq!(serde_json::to_value(&every).unwrap()["open_faces"], json!([{}]));
}

#[test]
fn a_file_with_picked_faces_reads_and_writes_as_it_did() {
    let saved = json!([picked()]);
    let shell = read(saved.clone()).unwrap();
    assert_eq!(shell.open_faces.picked().len(), 1);
    assert!(!shell.open_faces.is_all());
    assert_eq!(serde_json::to_value(&shell).unwrap()["open_faces"], saved);
}

#[test]
fn every_face_stands_alone_and_nothing_else_reads_as_it() {
    assert!(read(json!([{}, {}])).is_err());
    assert!(read(json!([{}, picked()])).is_err());
    assert!(read(json!([picked(), {}])).is_err());
    assert!(read(json!([{"feature": 3}])).is_err(), "a damaged reference is not every face");
    assert!(read(json!([{"label": "Boss-Extrude1"}])).is_err());
    assert!(read(json!("all")).is_err());
}
