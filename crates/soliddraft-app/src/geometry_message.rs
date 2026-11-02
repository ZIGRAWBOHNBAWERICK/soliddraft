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
    } else if message.contains("measure.unsupported-surface") {
        "Volume and mass are unavailable for this shape. You can still save the part and export a mesh."
    } else if message.contains("distance.unsupported-envelope") || message.contains("distance.cylinder-domain") {
        "This shape cannot be shelled. Use a convex tray with separated through bores and open its top or bottom face."
    } else if message.contains("step.unsupported-geometry")
        || (message.contains("export_step") && message.contains("unsupported"))
    {
        "STEP export is unavailable for this shape. Save the part or export a mesh instead."
    } else if message.contains("fillet.") {
        "This fillet cannot be made on the selected edges. Reduce the radius or select different edges."
    } else if message.contains("mass_properties") {
        "These properties could not be calculated. Try again or choose another feature."
    } else if message.contains("Catallax") || message.contains("kernel unavailable") || message.contains("numbers.") {
        "This operation is unavailable for the selected shape."
    } else {
        message
    }
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
            "distance.unsupported-envelope",
            "export_step: unsupported surface",
            "step.unsupported-geometry",
            "mass_properties failed",
            "numbers.operation-budget",
            "kernel unavailable",
        ] {
            assert!(!for_user(code).contains(code));
            let diagnostic = format!("{code}: refused (Catallax stage 3)");
            let shown = for_user(&diagnostic);
            assert!(!shown.contains(code) && !shown.contains("Catallax") && !shown.contains("stage"));
            assert!(!shown.is_empty());
        }
        assert_eq!(for_user("The radius must be more than 0 mm"), "The radius must be more than 0 mm");
    }
}
