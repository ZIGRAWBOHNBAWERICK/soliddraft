//! Geometry refusals in the interface. Diagnostics retain the original reason.

/// What the user reads for a kernel or backend `message`: plain words, never a code.
pub fn for_user(message: &str) -> String {
    let (message, reaches) = match message.split_once(soliddraft_model::RUNS_INTO) {
        Some((message, faces)) => (message, Some(faces.trim())),
        None => (message, None),
    };
    if let Some(reaches) = reaches.filter(|faces| !faces.is_empty()) {
        return match radius_limit(message) {
            Some(limit) => format!("This fillet runs into {reaches}. Try a radius under {limit} mm."),
            None => format!("This fillet runs into {reaches}. Reduce the radius or round fewer edges."),
        };
    }
    if message.contains("fillet.radius-too-large") {
        return match radius_limit(message) {
            Some(limit) => format!("The radius is too large for this edge. Try a radius under {limit} mm."),
            None => "The radius is too large for this edge. Try a smaller radius.".into(),
        };
    }
    // When the exact construction and the general one both refused, the general one's
    // reason is the one that holds: the exact one may only lack a construction.
    if let Some((_, general)) = message.split_once(NUMERICAL_REFUSED) {
        return plain_message(general).to_owned();
    }
    plain_message(message).to_owned()
}

/// What a refusal says before the general fillet construction's own reason.
const NUMERICAL_REFUSED: &str = "numerical continuation refused ";

/// The smallest radius limit a fillet refusal states, as the kernel wrote it.
fn radius_limit(message: &str) -> Option<&str> {
    message
        .split("; radius limit ")
        .skip(1)
        .filter_map(|rest| rest.split_once(" mm").map(|(n, _)| n))
        .filter(|n| n.parse::<f64>().is_ok_and(|n| n.is_finite() && n > 0.0))
        .min_by(|a, b| a.parse::<f64>().unwrap().total_cmp(&b.parse::<f64>().unwrap()))
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
        overflow(message)
    } else if message.contains("fillet.unsupported-contact") {
        "This edge and radius cannot be rounded together. Try a smaller radius or a different edge."
    } else if message.contains("fillet.unsupported-topology") {
        "This selection does not define a supported sharp edge. Choose a different edge."
    } else if message.contains("features.numerical-fillet-network")
        || message.contains("features.numerical-fillet-closure")
        || message.contains("fillet.numerical-chain")
    {
        "Rounding these edges together is not supported yet. Select different edges."
    } else if message.contains("fillet.numerical-contact") {
        "The fillet cannot roll along this edge at this radius. Try a different radius or select different edges."
    } else if message.contains("features.numerical-") || message.contains("fillet.numerical-") {
        "This fillet cannot be made on the selected edges. Reduce the radius or select different edges."
    } else if message.contains("recipe.ambiguous-reference") {
        "This selection matches more than one edge or face. Pick the intended edge or face again."
    } else if message.contains("draft.parallel-face-normal") {
        "Draft cannot tilt a face that lies across the pull. Pick side faces."
    } else if message.contains("draft.") && ["bore", "annul", "opening", "hole"].iter().any(|w| message.contains(w)) {
        "Draft cannot tilt this body yet: it has more than one opening along the pull."
    } else if message.contains("draft.") {
        "Draft cannot tilt this body yet. Try other faces, or draft before the cuts."
    } else if message.contains("mesh.precision-or-budget") {
        "This shape could not be displayed. Try suppressing a feature."
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
    } else if message.contains("fillet failed") && message.contains("intersect.") {
        // The kernel could not cut the rounded face where it meets the faces around it.
        "The fillet could not be trimmed where it meets the faces around it. Try a different radius or select \
         different edges."
    } else if message.contains("fillet.") {
        "This fillet cannot be made on the selected edges. Reduce the radius or select different edges."
    } else if message.contains("numbers.operation-budget") {
        "This operation reached its calculation limit. Try a simpler shape or suppress a feature."
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

/// `fillet.unsupported-overflow` names several refusals, and only some of them find the
/// fillet on another face. Others find rounds of the same feature running into each
/// other, a face the kernel cannot prove clear, or a contact that runs off its own face
/// or edge, so the explanation follows the reason given.
fn overflow(message: &str) -> &'static str {
    let says = |phrases: &[&str]| phrases.iter().any(|p| message.contains(p));
    if says(&[
        "blend carriers meet",
        "two strips",
        "two blend bands",
        "another blend",
        "junction affected regions",
        "meets another corner",
        "consume a strip",
    ]) {
        "The rounded edges run into each other. Reduce the radius or round fewer edges."
    } else if says(&["reaches a neighbouring face", "intersects the blend carrier", "meets the corner's support"]) {
        "This fillet reaches another face. Reduce the radius or round fewer edges."
    } else if says(&["not proved clear", "reaches the hull", "trimmed against"]) {
        "This fillet may run into another face. Reduce the radius or round fewer edges."
    } else {
        "This fillet would have to continue past the faces beside its edge, which is not supported yet. Reduce the \
         radius or select different edges."
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
        assert_eq!(
            for_user("export_step failed: numbers.operation-budget: operation work allowance exhausted"),
            "This operation reached its calculation limit. Try a simpler shape or suppress a feature."
        );
    }

    /// A fillet that would run into other faces names them as the tree does, with the
    /// largest radius that fits when the kernel found one, and never a code.
    #[test]
    fn a_fillet_that_reaches_other_faces_names_them_and_the_radius_that_fits() {
        let refusal = "fillet failed in the kernel: fillet.radius-too-large: step op_9: the ball reaches a face; \
                       radius limit 3.999 mm";
        assert_eq!(
            for_user(&format!("{refusal}{}Boss-Extrude1 · Side of line 2", soliddraft_model::RUNS_INTO)),
            "This fillet runs into Boss-Extrude1 · Side of line 2. Try a radius under 3.999 mm."
        );
        assert_eq!(
            for_user(&format!(
                "fillet failed in the kernel: fillet.unsupported-overflow: a face{}Cut-Extrude1 · Floor and \
                 Boss-Extrude1 · End cap",
                soliddraft_model::RUNS_INTO
            )),
            "This fillet runs into Cut-Extrude1 · Floor and Boss-Extrude1 · End cap. Reduce the radius or round fewer \
             edges."
        );
        // Of two stated limits, the smaller one fits both constructions.
        assert_eq!(
            for_user(
                "fillet.radius-too-large: exact path refused fillet.radius-too-large: a; radius limit 5 mm; numerical \
                 continuation refused fillet.radius-too-large: b; radius limit 4.5 mm"
            ),
            "The radius is too large for this edge. Try a radius under 4.5 mm."
        );
    }

    /// When the exact construction and the general one both refuse, the user reads the
    /// general one's reason: the exact one's may only say it has no construction.
    #[test]
    fn the_general_fillet_construction_gives_the_reason() {
        let both = "fillet failed in the kernel (status 0): features.numerical-fillet-network: step op_96: \
                    features.numerical-fillet-network: exact path refused fillet.unsupported-overflow: a non-end \
                    neighbour intersects the blend carrier; numerical continuation refused \
                    features.numerical-fillet-network: the retained spline assembly needs one outer-boundary edge";
        assert_eq!(for_user(both), "Rounding these edges together is not supported yet. Select different edges.");
        for code in ["features.numerical-fillet-source", "fillet.numerical-body", "features.numerical-fillet-name"] {
            let shown = for_user(&format!("fillet failed in the kernel (status 0): {code}: detail"));
            assert_eq!(
                shown,
                "This fillet cannot be made on the selected edges. Reduce the radius or select different edges."
            );
        }
    }

    /// A fillet the kernel cannot trim against its neighbours says so, rather than the
    /// generic text every other kernel failure gets.
    #[test]
    fn an_intersection_refusal_inside_a_fillet_names_the_trim() {
        let trim = "The fillet could not be trimmed where it meets the faces around it. Try a different radius or \
                    select different edges.";
        for diagnostic in [
            "fillet failed in the kernel (status 0): intersect.unsupported-pair: step 3: intersect.unsupported-pair: \
             surface pair is outside piece 4 coverage",
            "fillet failed in the kernel (status 0): intersect.fillet-cut-plane: step 2: intersect.fillet-cut-plane: \
             the cut does not meet its plane",
        ] {
            assert_eq!(for_user(diagnostic), trim, "{diagnostic}");
        }
        assert_eq!(
            for_user("chamfer failed in the kernel (status 0): intersect.unsupported-pair: no coverage"),
            "This operation is unavailable for the selected shape."
        );
    }

    /// Only the overflow refusal that finds another face says the fillet reaches one.
    #[test]
    fn overflow_says_another_face_only_when_one_is_reached() {
        let at = |detail: &str| {
            for_user(&format!(
                "fillet failed in the kernel (status 0): fillet.unsupported-overflow: step 2: \
                 fillet.unsupported-overflow: {detail}"
            ))
        };
        for reached in [
            "variable contact region reaches a neighbouring face and needs continuation",
            // A fillet along a plate edge whose radius cuts into a hole beside it.
            "a non-end neighbour intersects the blend carrier; its trimmed contact continuation must be resolved \
             before publishing; maximum unknown",
            "a non-incident face meets the corner's support; its trimmed continuation must be resolved before publishing",
        ] {
            assert_eq!(at(reached), "This fillet reaches another face. Reduce the radius or round fewer edges.");
        }
        for crossing in [
            "nonincident blend carriers meet and require simultaneous contact-region trimming",
            "two blend bands are not proved apart and need trimmed continuation",
            "a junction's separation from another blend is unproved",
        ] {
            assert_eq!(at(crossing), "The rounded edges run into each other. Reduce the radius or round fewer edges.");
        }
        for unproved in [
            "a face reaches the hull of the blend band and needs trimmed continuation",
            "a non-incident face is not proved clear of the corner sphere",
            "a crossing band is not proved clear of another input support",
            "a void inside the inscribed shell needs its blend region trimmed against it",
        ] {
            assert_eq!(at(unproved), "This fillet may run into another face. Reduce the radius or round fewer edges.");
        }
        for nothing_reached in [
            "a contact rail leaves its end-wall edge and needs neighbour continuation",
            "the deeper strip at a vertex runs past the unselected edge and needs neighbour continuation",
            "an offset junction endpoint is not strictly inside its selected run",
            "a crossing contact leaves its input face",
            "a contact rail passes the round's rail outside its edge and needs neighbour continuation",
            "rail contact is not a circle",
        ] {
            let shown = at(nothing_reached);
            assert!(!shown.contains("another face"), "{nothing_reached}: {shown}");
            assert_eq!(
                shown,
                "This fillet would have to continue past the faces beside its edge, which is not supported yet. \
                 Reduce the radius or select different edges."
            );
        }
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
            "mesh.precision-or-budget",
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
