//! Geometry refusals in the interface. Diagnostics retain the original reason.

pub(crate) fn for_user(message: &str) -> &str {
    if message.contains("fillet.unsupported-overflow") {
        "This fillet reaches another face. Reduce the radius or round fewer edges."
    } else if message.contains("fillet.unsupported-contact") {
        "This edge and radius cannot be rounded together. Try a smaller radius or a different edge."
    } else if message.contains("fillet.unsupported-topology") {
        "This fillet needs a closed edge without corners. Choose a different edge."
    } else if message.contains("recipe.ambiguous-reference") {
        "This selection matches more than one edge or face. Pick the intended edge or face again."
    } else if has_code(message, "measure") || has_code(message, "iges") {
        // Measurements and IGES output the kernel does not cover yet.
        "Coming soon"
    } else if message.contains("distance.unsupported-envelope") || message.contains("distance.cylinder-domain") {
        "This shape cannot be shelled. Use a convex tray with separated through bores and open its top or bottom face."
    } else if message.contains("step.unsupported-geometry")
        || message.contains("numbers.geometric-gap-unproven")
        || (message.contains("export_step") && message.contains("unsupported"))
    {
        "Coming soon"
    } else if message.contains("fillet.") {
        "This fillet cannot be made on the selected edges. Reduce the radius or select different edges."
    } else if message.contains("mass_properties") {
        "These properties could not be calculated. Try again or choose another feature."
    } else if message.contains("Catallax")
        || message.contains("kernel")
        || message.contains("intersect.")
        || message.contains("numbers.")
    {
        "This operation is unavailable for the selected shape."
    } else {
        message
    }
}

/// Whether `message` carries a diagnostic code of `family`, such as
/// `measure.nonplanar-face`: the family, a dot and hyphenated lowercase words. A file
/// name such as `measure.step` is not one.
fn has_code(message: &str, family: &str) -> bool {
    message.match_indices(family).any(|(at, _)| {
        let before = message[..at].chars().next_back();
        let code = message[at + family.len()..].strip_prefix('.').unwrap_or("");
        let word: String = code.chars().take_while(|c| c.is_ascii_lowercase() || *c == '-').collect();
        !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') && word.contains('-') && !word.starts_with('-')
    })
}

#[cfg(test)]
mod tests {
    use super::for_user;

    #[test]
    fn refusals_explain_the_next_action_without_internal_names() {
        for code in [
            "fillet.unsupported-contact",
            "fillet.unsupported-overflow",
            "fillet.unsupported-topology",
            "recipe.ambiguous-reference",
            "measure.unsupported-surface",
            "measure.unsupported-pair",
            "measure.nonplanar-face",
            "measure.unsupported-curve",
            "iges.uncertified-model",
            "distance.unsupported-envelope",
            "export_step: unsupported surface",
            "step.unsupported-geometry",
            "numbers.geometric-gap-unproven",
            "mass_properties failed",
            "numbers.operation-budget",
            "kernel unavailable",
            "intersect.unsupported-pair",
            "fillet failed in the kernel: unsupported surface",
        ] {
            assert!(!for_user(code).contains(code));
            let diagnostic = format!("{code}: refused (Catallax stage 3)");
            let shown = for_user(&diagnostic);
            assert!(!shown.contains(code) && !shown.contains("Catallax") && !shown.contains("stage"));
            assert!(!shown.is_empty());
        }
        assert_eq!(for_user("The radius must be more than 0 mm"), "The radius must be more than 0 mm");
        // Texts the real-use pass saw in the Measure panel and the status bar.
        for seen in [
            "Hole1 · Hole 1 wall and Hole2 · Hole 1 wall could not be measured: parallel_face_to_face failed \
             (status 0): measure.nonplanar-face: face #94 is not planar",
            "Fillet1 · Edge 1 and Boss-Extrude2 · End cap could not be measured: unsupported: \
             measure.unsupported-pair: minimum_distance is not supported between Edge(EdgeId(29)) and Face(FaceId(15))",
            "Export: export_iges failed (status 0): iges.uncertified-model: the model lacks the \
             exact-definition correspondence the writer needs",
        ] {
            assert_eq!(for_user(seen), "Coming soon", "{seen}");
        }
        let file = "cannot read /parts/measure.step: No such file or directory (os error 2)";
        assert_eq!(for_user(file), file);
    }
}
