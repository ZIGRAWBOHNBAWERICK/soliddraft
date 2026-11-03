//! Geometry refusals in the interface. Diagnostics retain the original reason.

pub(crate) fn for_user(message: &str) -> String {
    if message.contains("fillet.radius-too-large") {
        if let Some(limit) = message.split("; radius limit ").nth(1).and_then(|s| s.split_once(" mm").map(|(n, _)| n))
            && limit.parse::<f64>().is_ok_and(|n| n.is_finite() && n > 0.0)
        {
            return format!("The radius is too large for this edge. Try a radius under {limit} mm.");
        }
        return "The radius is too large for this edge. Try a smaller radius.".into();
    }
    plain_message(message).to_owned()
}

fn plain_message(message: &str) -> &str {
    if message.trim_end().ends_with("is coming soon.") {
        // Unfinished work says only Coming soon, not which part of it is unfinished.
        "Coming soon"
    } else if message.contains("fillet.unsupported-end") {
        "Fillets ending here are not supported yet."
    } else if message.contains("fillet.unsupported-corner") {
        "Fillets meeting at this corner are not supported yet."
    } else if message.contains("fillet.contact-off-face") {
        "The fillet would leave the selected faces. Reduce the radius or choose different edges."
    } else if message.contains("fillet.unsupported-overflow") {
        "This fillet reaches another face. Reduce the radius or round fewer edges."
    } else if message.contains("fillet.unsupported-contact") {
        "This edge and radius cannot be rounded together. Try a smaller radius or a different edge."
    } else if message.contains("fillet.unsupported-topology") {
        "This selection does not define a supported sharp edge. Choose a different edge."
    } else if message.contains("recipe.ambiguous-reference") {
        "This selection matches more than one edge or face. Pick the intended edge or face again."
    } else if has_code(message, "measure") || has_code(message, "iges") {
        // Measurements and IGES output the kernel does not cover yet.
        "Coming soon"
    } else if message.contains("distance.unsupported-envelope") || message.contains("distance.cylinder-domain") {
        "This shell arrangement is not supported yet. Try different openings or a smaller wall thickness."
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

    /// The real-use pass read "Catallax drafts planar faces ..." under the Draft editor and
    /// "... through the kernel" under a drawing view. Hints, labels, tooltips and buttons
    /// written in the source name no kernel, stage or development phase.
    #[test]
    fn interface_texts_name_no_kernel_stage_or_phase() {
        const CALLS: [&str; 7] = [
            "hint(ui, \"",
            "ui.label(\"",
            ".on_hover_text(\"",
            ".on_disabled_hover_text(\"",
            ".hint_text(\"",
            "Button::new(\"",
            "RichText::new(\"",
        ];
        let mut found = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for call in CALLS {
                    for (at, _) in text.match_indices(call) {
                        let rest = &text[at + call.len()..];
                        let literal = &rest[..rest.find('"').unwrap_or(rest.len())];
                        let lower = literal.to_lowercase();
                        let phase = ["stage ", "phase "].iter().any(|w| {
                            lower
                                .match_indices(w)
                                .any(|(i, _)| lower[i + w.len()..].starts_with(|c: char| c.is_ascii_digit()))
                        });
                        if lower.contains("catallax") || lower.contains("kernel") || phase {
                            found.push(format!("{}: {literal}", path.display()));
                        }
                    }
                }
            }
        }
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn radius_refusal_keeps_the_proved_limit_and_strict_advice() {
        assert_eq!(
            for_user("fillet.radius-too-large: contact limit; radius limit 1 mm"),
            "The radius is too large for this edge. Try a radius under 1 mm."
        );
        for limit in ["NaN", "inf", "-1", "0", "no limit"] {
            let shown = for_user(&format!("fillet.radius-too-large: contact limit; radius limit {limit} mm"));
            assert_eq!(shown, "The radius is too large for this edge. Try a smaller radius.");
        }
        assert_eq!(
            for_user("fillet.unsupported-corner: equal crossing pole"),
            "Fillets meeting at this corner are not supported yet."
        );
    }

    #[test]
    fn refusals_explain_the_next_action_without_internal_names() {
        for code in [
            "fillet.radius-too-large",
            "fillet.unsupported-end",
            "fillet.unsupported-corner",
            "fillet.contact-off-face",
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
        for unfinished in ["This sweep path is coming soon.", "Building on an imported body is coming soon."] {
            assert_eq!(for_user(unfinished), "Coming soon");
        }
        let file = "cannot read /parts/measure.step: No such file or directory (os error 2)";
        assert_eq!(for_user(file), file);
    }
}
