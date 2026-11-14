//! Generates the several-solid IGES fixture from the reviewed STEP fixture.

use catallax_numbers::Decider;
use std::io::Write;

fn main() {
    let bytes = include_bytes!("../../../ci/fixtures/import/two-blocks-5um.step");
    let source = catallax_import::SourceFormat::Step.read(bytes).unwrap();
    let mut model = catallax_topology::Model::new();
    let bodies = catallax_import::build_file(&mut model, &source, None).unwrap();
    let mut bytes = Vec::new();
    catallax_iges::write_iges(&model, &bodies, &Default::default(), &mut Decider::default(), &mut bytes).unwrap();
    println!("BEGIN_IGES");
    std::io::stdout().write_all(&bytes).unwrap();
    println!("END_IGES");
}
