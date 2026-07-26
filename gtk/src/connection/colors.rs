//! Connection color ids shared with the Svelte `CONNECTION_COLORS` palette.

/// Hex palette used for group color pickers (Svelte `GROUP_COLORS` parity).
pub const GROUP_COLOR_HEXES: &[&str] = &[
    "#ef4444", "#f97316", "#eab308", "#22c55e", "#14b8a6", "#3b82f6", "#8b5cf6", "#ec4899",
    "#64748b",
];

/// Resolve a stored `color_id` (or hex fallback) to a CSS class suffix / hex.
pub fn color_hex(color_id: &str) -> &'static str {
    match color_id {
        "red" => "#ef4444",
        "orange" => "#f97316",
        "yellow" => "#eab308",
        "green" => "#22c55e",
        "teal" => "#14b8a6",
        "blue" => "#3b82f6",
        "violet" => "#8b5cf6",
        "pink" => "#ec4899",
        "slate" => "#64748b",
        "white" => "#f8fafc",
        _ => "#64748b",
    }
}

/// CSS class for a connection color swatch (`connection-color-red`, …).
pub fn color_css_class(color_id: &str) -> &'static str {
    match color_id {
        "red" => "connection-color-red",
        "orange" => "connection-color-orange",
        "yellow" => "connection-color-yellow",
        "green" => "connection-color-green",
        "teal" => "connection-color-teal",
        "blue" => "connection-color-blue",
        "violet" => "connection-color-violet",
        "pink" => "connection-color-pink",
        "white" => "connection-color-white",
        // Unknown ids and "slate" share the slate swatch.
        _ => "connection-color-slate",
    }
}

/// Map a group hex color (or None) onto the nearest named swatch class.
pub fn group_color_css_class(hex: Option<&str>) -> &'static str {
    let Some(hex) = hex else {
        return "connection-color-slate";
    };
    let normalized = hex.trim().to_ascii_lowercase();
    for id in [
        "red", "orange", "yellow", "green", "teal", "blue", "violet", "pink", "slate", "white",
    ] {
        if color_hex(id).eq_ignore_ascii_case(&normalized) {
            return color_css_class(id);
        }
    }
    "connection-color-slate"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_ids_map() {
        assert_eq!(color_hex("blue"), "#3b82f6");
        assert_eq!(color_css_class("pink"), "connection-color-pink");
        assert_eq!(color_css_class("nope"), "connection-color-slate");
    }

    #[test]
    fn group_hex_maps_to_named_class() {
        assert_eq!(
            group_color_css_class(Some("#ef4444")),
            "connection-color-red"
        );
        assert_eq!(group_color_css_class(None), "connection-color-slate");
    }
}
