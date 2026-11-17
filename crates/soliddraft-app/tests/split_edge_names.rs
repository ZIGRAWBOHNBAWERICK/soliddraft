//! An edge a later feature split into pieces between the same two faces is named piece by
//! piece, by the faces at its ends, and a feature on one piece survives save and reopen.

use soliddraft_app::document::{Command, Document};

fn bracket() -> Document {
    let bytes = include_bytes!("../../../site/parts/bracket.dwprt");
    soliddraft_app::file::open_bytes("bracket.dwprt", bytes, soliddraft_app::kernel(), None).unwrap()
}

/// The `--demo` sample part, a plate with a flange and a boss, names 46 of its 48 edges,
/// each finding its edge again; on main 26 had no name. The other two are pieces between
/// the same faces that also end at the same faces, so nothing stable tells them apart and
/// they stay unnamed rather than being told apart by place.
#[test]
fn the_demo_part_names_every_edge_its_end_faces_single_out() {
    let doc = soliddraft_app::demo_document();
    let body = doc.model().body().unwrap();
    let topology = doc.model().kernel().topology(body).unwrap();
    assert_eq!(topology.edges.len(), 48);
    let mut unnamed = Vec::new();
    for edge in &topology.edges {
        match doc.model().edge_ref(edge.id) {
            Ok(named) => assert_eq!(doc.model().resolve_edge(&named, body).unwrap(), edge.id, "{}", named.label),
            Err(why) => unnamed.push(why.to_string()),
        }
    }
    assert_eq!(unnamed.len(), 2, "{unnamed:?}");
    assert!(unnamed.iter().all(|why| why.contains("end at the faces it ended at")), "{unnamed:?}");
}

/// In the sample bracket two countersinks split the edge between the base's end cap and
/// the side of line 2 into three pieces. Each piece has its own name and label, and every
/// edge of the bracket has a name.
#[test]
fn each_piece_of_a_split_edge_has_its_own_name() {
    let doc = bracket();
    let body = doc.model().body().unwrap();
    let topology = doc.model().kernel().topology(body).unwrap();
    let mut pieces = Vec::new();
    for edge in &topology.edges {
        let named = doc.model().edge_ref(edge.id).unwrap_or_else(|e| panic!("{} has no name: {e}", edge.id));
        assert_eq!(doc.model().resolve_edge(&named, body).unwrap(), edge.id, "{}", named.label);
        if named.piece.is_some() {
            pieces.push(named);
        }
    }
    let mut labels: Vec<&str> = pieces.iter().map(|p| &*p.label).collect();
    labels.sort();
    let shared = "Boss-Extrude1 · End cap ∩ Boss-Extrude1 · Side of line 2 · ends at ";
    assert_eq!(
        labels,
        [
            format!("{shared}Boss-Extrude1 · Side of line 1, Hole1 · Hole 3 countersink"),
            format!("{shared}Boss-Extrude1 · Side of line 3, Hole1 · Hole 4 countersink"),
            format!("{shared}Hole1 · Hole 3 countersink, Hole1 · Hole 4 countersink"),
        ]
    );
}

/// Each piece's saved reference, read back in the reopened part, finds that same piece.
#[test]
fn a_saved_piece_reference_finds_its_piece_after_reopening() {
    let doc = bracket();
    let body = doc.model().body().unwrap();
    let topology = doc.model().kernel().topology(body).unwrap();
    let pieces: Vec<_> = topology
        .edges
        .iter()
        .filter_map(|e| doc.model().edge_ref(e.id).ok().filter(|r| r.piece.is_some()).map(|r| (e.midpoint, r)))
        .collect();
    assert_eq!(pieces.len(), 3);
    let saved = doc.model().to_bytes().unwrap();
    let reopened = soliddraft_app::file::open_bytes("bracket.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
    let again = reopened.model().body().unwrap();
    let edges = reopened.model().kernel().topology(again).unwrap().edges;
    for (midpoint, piece) in pieces {
        let stored: soliddraft_model::EdgeRef = serde_json::from_str(&serde_json::to_string(&piece).unwrap()).unwrap();
        assert_eq!(stored, piece);
        let found = reopened.model().resolve_edge(&stored, again).unwrap();
        let at = edges.iter().find(|e| e.id == found).unwrap().midpoint;
        assert!((at - midpoint).length() < 1e-6, "{} found an edge at {at:?}, not {midpoint:?}", piece.label);
    }
}

/// A piece's reference never rebinds to the edge that remains when the cut that split it
/// is gone. Suppressing the countersinks leaves one whole edge between the end cap and
/// the side of line 2; each piece's saved reference asks for repair instead of taking it.
#[test]
fn a_piece_reference_needs_repair_when_its_split_is_gone() {
    let mut doc = bracket();
    let body = doc.model().body().unwrap();
    let topology = doc.model().kernel().topology(body).unwrap();
    let pieces: Vec<_> =
        topology.edges.iter().filter_map(|e| doc.model().edge_ref(e.id).ok().filter(|r| r.piece.is_some())).collect();
    assert_eq!(pieces.len(), 3);
    let holes = doc.features.iter().find(|f| f.name == "Hole1").unwrap().id;
    doc.apply(Command::Suppress { id: holes, suppressed: true }).unwrap();
    let whole = doc.model().body().unwrap();
    let edges = doc.model().kernel().topology(whole).unwrap().edges;
    let between = edges
        .iter()
        .filter(|e| {
            doc.model()
                .edge_ref(e.id)
                .is_ok_and(|r| r.label == "Boss-Extrude1 · End cap ∩ Boss-Extrude1 · Side of line 2".into())
        })
        .count();
    assert_eq!(between, 1, "suppressing the countersinks leaves one whole edge");
    for piece in pieces {
        let found = doc.model().resolve_edge(&piece, whole);
        assert!(found.is_err(), "{} rebound to {:?}", piece.label, found);
    }
}

/// A piece whose end faces no longer single it out is never bound by its place: with the
/// three pieces all present, a reference that has lost its end faces asks for repair and
/// offers the pieces, rather than taking one of them.
#[test]
fn a_piece_its_end_faces_do_not_single_out_asks_for_repair() {
    let doc = bracket();
    let body = doc.model().body().unwrap();
    let topology = doc.model().kernel().topology(body).unwrap();
    let named =
        topology.edges.iter().find_map(|e| doc.model().edge_ref(e.id).ok().filter(|r| r.piece.is_some())).unwrap();
    let mut bare = named.clone();
    bare.piece = Some(Box::new(soliddraft_model::EdgePiece { ends: Vec::new() }));
    let repair = doc.model().resolve_edge(&bare, body).unwrap_err();
    assert_eq!(repair.candidates.len(), 3, "{repair:?}");
}
