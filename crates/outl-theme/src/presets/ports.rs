//! Ports of other editors' and terminals' colour schemes.
//!
//! Split out of `presets.rs` on the question "whose look is this?" —
//! the sibling module holds outl's own brand palettes plus the two
//! generic ANSI/light fallbacks, and these six reproduce a scheme
//! somebody else designed. Every one is re-exported from
//! [`crate::presets`], so the module boundary is not part of the
//! public API: callers still say `presets::dracula()`.
//!
//! Each `fn` here cites where its hex values came from, because that
//! is the only thing that makes "is this still Dracula?" answerable.

use crate::Palette;

/// Logseq light — the default light theme of Logseq. White canvas,
/// warm dark-gray text (`#433f38`), and the Blueprint.js blue rail
/// (`#106ba3` links, `#0069b6` active accent) Logseq ships out of
/// the box. Hex values sourced from Logseq's `vars-classic.css`
/// (`--ls-*` custom properties); the dim grays are alpha blends of
/// the primary text color on white, matching how Logseq fades
/// bullets and metadata.
pub fn logseq_light() -> Palette {
    let bg = "#ffffff"; // --ls-primary-background-color
    let bg_elev = "#f7f7f7"; // --ls-secondary-background-color
    let fg = "#433f38"; // --ls-primary-text-color
    let fg_strong = "#161e2e"; // --ls-secondary-text-color
    let title = "#0f1419"; // --ls-title-text-color
    let fg_dim = "#8e8b87"; // fg at 60% on white
    let fg_dimmer = "#bdbbb9"; // fg at 35% on white
    let border = "#cccccc"; // --ls-border-color
    let blue = "#106ba3"; // --ls-link-text-color
    let blue_deep = "#1a537c"; // --ls-link-text-hover-color
    let blue_bright = "#0069b6"; // --ls-active-primary-color
    let selection = "#e4f2ff"; // --ls-selection-background-color
    let green = "#0d8050"; // Blueprint green2, in-family with blue2
    let orange = "#bf7326"; // Blueprint orange2
    let violet = "#752f75"; // Blueprint violet2

    Palette {
        name: "logseq-light".into(),
        bg: bg.into(),
        bg_elev: bg_elev.into(),
        fg: fg.into(),
        fg_dim: "#726f6c".into(), // 5.0:1 on bg, 4.7:1 on bg_elev
        fg_dimmer: fg_dimmer.into(),
        border: border.into(),
        hint: fg_dim.into(),
        accent: blue.into(),
        accent_fg: bg.into(), // the canvas itself, 5.75:1
        accent_soft: blue_bright.into(),
        accent_alt: green.into(),
        warn: orange.into(),
        destructive: "#c5372c".into(), // Blueprint red, matches its link blue
        destructive_fg: "#ffffff".into(), // the canvas itself, 5.31:1
        ref_link_fg: blue.into(),
        tag_link_fg: blue_deep.into(),
        md_link_fg: blue.into(),
        bold_fg: fg_strong.into(),
        italic_fg: fg.into(),
        strike_fg: fg_dim.into(),
        highlight_bg: "#fff3a0".into(),
        highlight_fg: fg.into(),
        code_fg: green.into(),
        todo_open_fg: orange.into(),
        todo_done_fg: green.into(),
        todo_done_body_fg: fg_dim.into(),
        property_key_fg: fg_dim.into(),
        property_value_fg: blue_bright.into(),
        heading_fg: title.into(),
        dim_fg: fg_dim.into(),
        selected_bullet_bg: blue.into(),
        selected_bullet_fg: bg.into(),
        cursor_block_bg: fg.into(),
        cursor_block_fg: bg.into(),
        cursor_caret_fg: blue_bright.into(),
        status_normal_bg: blue.into(),
        status_normal_fg: bg.into(),
        status_insert_bg: green.into(),
        status_insert_fg: bg.into(),
        status_visual_bg: violet.into(),
        status_visual_fg: bg.into(),
        status_message_fg: orange.into(),
        list_selected_bg: selection.into(),
        list_selected_fg: title.into(),
        help_title_fg: blue.into(),
    }
}

/// Dracula — popular dark palette (zenorocha).
pub fn dracula() -> Palette {
    let bg = "#282a36";
    let fg = "#f8f8f2";
    let comment = "#6272a4";
    let cyan = "#8be9fd";
    let green = "#50fa7b";
    let orange = "#ffb86c";
    let pink = "#ff79c6";
    let purple = "#bd93f9";
    let yellow = "#f1fa8c";

    Palette {
        name: "dracula".into(),
        bg: bg.into(),
        bg_elev: "#21222c".into(),
        fg: fg.into(),
        fg_dim: "#8692b9".into(), // comment lifted to 4.6:1 on bg
        fg_dimmer: comment.into(),
        border: comment.into(),
        hint: comment.into(),
        accent: cyan.into(),
        accent_fg: bg.into(), // the canvas itself, 10.29:1
        accent_soft: purple.into(),
        accent_alt: green.into(),
        warn: yellow.into(),
        destructive: "#ff5555".into(),    // Dracula "red"
        destructive_fg: "#21222c".into(), // Dracula bg_elev, 5.03:1 (plain bg is 4.53, too thin)
        ref_link_fg: cyan.into(),
        tag_link_fg: pink.into(),
        md_link_fg: purple.into(),
        bold_fg: orange.into(),
        italic_fg: yellow.into(),
        strike_fg: comment.into(),
        highlight_bg: yellow.into(),
        highlight_fg: bg.into(),
        code_fg: green.into(),
        todo_open_fg: yellow.into(),
        todo_done_fg: green.into(),
        todo_done_body_fg: comment.into(),
        property_key_fg: comment.into(),
        property_value_fg: purple.into(),
        heading_fg: pink.into(),
        dim_fg: comment.into(),
        selected_bullet_bg: cyan.into(),
        selected_bullet_fg: bg.into(),
        cursor_block_bg: fg.into(),
        cursor_block_fg: bg.into(),
        cursor_caret_fg: fg.into(),
        status_normal_bg: cyan.into(),
        status_normal_fg: bg.into(),
        status_insert_bg: green.into(),
        status_insert_fg: bg.into(),
        status_visual_bg: pink.into(),
        status_visual_fg: bg.into(),
        status_message_fg: yellow.into(),
        list_selected_bg: cyan.into(),
        list_selected_fg: bg.into(),
        help_title_fg: yellow.into(),
    }
}

/// Solarized Dark — Ethan Schoonover's classic muted palette.
pub fn solarized_dark() -> Palette {
    let base03 = "#002b36";
    let base02 = "#073642";
    let base01 = "#586e75";
    let base0 = "#839496";
    let base1 = "#93a1a1";
    let yellow = "#b58900";
    let orange = "#cb4b16";
    let red = "#dc322f";
    let magenta = "#d33682";
    let violet = "#6c71c4";
    let blue = "#268bd2";
    let cyan = "#2aa198";
    let green = "#859900";

    Palette {
        name: "solarized-dark".into(),
        bg: base03.into(),
        bg_elev: base02.into(),
        fg: base0.into(),
        fg_dim: "#7f9399".into(), // 4.7:1 on base03, 4:1 on base02 like base0
        fg_dimmer: base01.into(),
        border: base02.into(),
        hint: base1.into(),
        accent: cyan.into(),
        accent_fg: base03.into(), // the canvas itself, 4.75:1
        accent_soft: violet.into(),
        accent_alt: green.into(),
        warn: yellow.into(),
        destructive: red.into(),          // Solarized red
        destructive_fg: "#ffffff".into(), // 4.63:1; every Solarized base tone fails
        ref_link_fg: blue.into(),
        tag_link_fg: magenta.into(),
        md_link_fg: violet.into(),
        bold_fg: orange.into(),
        italic_fg: violet.into(),
        strike_fg: base01.into(),
        highlight_bg: yellow.into(),
        highlight_fg: base03.into(),
        code_fg: green.into(),
        todo_open_fg: red.into(),
        todo_done_fg: green.into(),
        todo_done_body_fg: base01.into(),
        property_key_fg: base01.into(),
        property_value_fg: cyan.into(),
        heading_fg: base1.into(),
        dim_fg: base01.into(),
        selected_bullet_bg: cyan.into(),
        selected_bullet_fg: base03.into(),
        cursor_block_bg: base1.into(),
        cursor_block_fg: base03.into(),
        cursor_caret_fg: base1.into(),
        status_normal_bg: cyan.into(),
        status_normal_fg: base03.into(),
        status_insert_bg: green.into(),
        status_insert_fg: base03.into(),
        status_visual_bg: magenta.into(),
        status_visual_fg: base03.into(),
        status_message_fg: yellow.into(),
        list_selected_bg: cyan.into(),
        list_selected_fg: base03.into(),
        help_title_fg: yellow.into(),
    }
}

/// Nord — arctic blue-greys, low contrast.
pub fn nord() -> Palette {
    let polar0 = "#2e3440";
    let polar1 = "#3b4252";
    let snow0 = "#d8dee9";
    let snow1 = "#e5e9f0";
    let snow2 = "#eceff4";
    let frost1 = "#88c0d0";
    let frost2 = "#81a1c1";
    let frost3 = "#5e81ac";
    let aurora_red = "#bf616a";
    let aurora_orange = "#d08770";
    let aurora_yellow = "#ebcb8b";
    let aurora_green = "#a3be8c";
    let aurora_purple = "#b48ead";

    Palette {
        name: "nord".into(),
        bg: polar0.into(),
        bg_elev: polar1.into(),
        fg: snow1.into(),
        fg_dim: snow0.into(),
        fg_dimmer: "#4c566a".into(),
        border: polar1.into(),
        hint: snow0.into(),
        accent: frost1.into(),
        accent_fg: polar0.into(), // the canvas itself, 6.24:1
        accent_soft: frost2.into(),
        accent_alt: aurora_green.into(),
        warn: aurora_yellow.into(),
        destructive: aurora_red.into(),   // nord11
        destructive_fg: "#000000".into(), // 5.13:1; nord0 is the darkest Nord tone and only reaches 3.05
        ref_link_fg: frost1.into(),
        tag_link_fg: aurora_purple.into(),
        md_link_fg: frost3.into(),
        bold_fg: aurora_orange.into(),
        italic_fg: frost2.into(),
        strike_fg: "#4c566a".into(),
        highlight_bg: aurora_yellow.into(),
        highlight_fg: polar0.into(),
        code_fg: aurora_green.into(),
        todo_open_fg: aurora_yellow.into(),
        todo_done_fg: aurora_green.into(),
        todo_done_body_fg: "#4c566a".into(),
        property_key_fg: "#4c566a".into(),
        property_value_fg: frost2.into(),
        heading_fg: snow2.into(),
        dim_fg: "#4c566a".into(),
        selected_bullet_bg: frost1.into(),
        selected_bullet_fg: polar0.into(),
        cursor_block_bg: snow1.into(),
        cursor_block_fg: polar0.into(),
        cursor_caret_fg: snow1.into(),
        status_normal_bg: frost1.into(),
        status_normal_fg: polar0.into(),
        status_insert_bg: aurora_green.into(),
        status_insert_fg: polar0.into(),
        status_visual_bg: aurora_purple.into(),
        status_visual_fg: polar0.into(),
        status_message_fg: aurora_red.into(),
        list_selected_bg: frost1.into(),
        list_selected_fg: polar0.into(),
        help_title_fg: aurora_yellow.into(),
    }
}

/// Monokai — Wimer Hazenberg's high-contrast palette.
pub fn monokai() -> Palette {
    let bg = "#272822";
    let fg = "#f8f8f2";
    let comment = "#75715e";
    let pink = "#f92672";
    let orange = "#fd971f";
    let yellow = "#e6db74";
    let green = "#a6e22e";
    let blue = "#66d9ef";
    let purple = "#ae81ff";

    Palette {
        name: "monokai".into(),
        bg: bg.into(),
        bg_elev: "#1e1f1c".into(),
        fg: fg.into(),
        fg_dim: "#939081".into(), // comment lifted to 4.6:1 on bg
        fg_dimmer: comment.into(),
        border: comment.into(),
        hint: comment.into(),
        accent: pink.into(),
        accent_fg: "#000000".into(), // 5.55:1; the canvas reaches only 3.93 on this pink
        accent_soft: purple.into(),
        accent_alt: green.into(),
        warn: yellow.into(),
        destructive: pink.into(),         // Monokai pink
        destructive_fg: "#141511".into(), // 4.85:1; darker than Monokai bg, which reaches only 3.93
        ref_link_fg: blue.into(),
        tag_link_fg: pink.into(),
        md_link_fg: blue.into(),
        bold_fg: orange.into(),
        italic_fg: yellow.into(),
        strike_fg: comment.into(),
        highlight_bg: yellow.into(),
        highlight_fg: bg.into(),
        code_fg: green.into(),
        todo_open_fg: pink.into(),
        todo_done_fg: green.into(),
        todo_done_body_fg: comment.into(),
        property_key_fg: comment.into(),
        property_value_fg: purple.into(),
        heading_fg: pink.into(),
        dim_fg: comment.into(),
        selected_bullet_bg: pink.into(),
        selected_bullet_fg: bg.into(),
        cursor_block_bg: fg.into(),
        cursor_block_fg: bg.into(),
        cursor_caret_fg: fg.into(),
        status_normal_bg: pink.into(),
        status_normal_fg: bg.into(),
        status_insert_bg: green.into(),
        status_insert_fg: bg.into(),
        status_visual_bg: purple.into(),
        status_visual_fg: bg.into(),
        status_message_fg: yellow.into(),
        list_selected_bg: pink.into(),
        list_selected_fg: bg.into(),
        help_title_fg: yellow.into(),
    }
}

/// Gruvbox Dark — morhetz's retro-groove palette. Warm charcoal
/// background (`#282828`), cream text (`#ebdbb2`), and the classic
/// muted yellow / green / aqua / purple accents. Hex values sourced
/// from the canonical gruvbox palette (dark variant).
pub fn gruvbox() -> Palette {
    let bg = "#282828"; // bg0
    let bg_elev = "#3c3836"; // bg1
    let border = "#504945"; // bg2
    let fg = "#ebdbb2"; // fg1
    let fg_soft = "#d5c4a1"; // fg2
    let fg_dim = "#a89984"; // fg4
    let fg_dimmer = "#7c6f64"; // fg5
    let yellow = "#fabd2f";
    let orange = "#fe8019";
    let green = "#b8bb26";
    let aqua = "#8ec07c";
    let blue = "#458588";
    let purple = "#d3869b";
    let red = "#fb4934"; // bright red — #cc241d is too dim on bg0 for a destructive hue

    Palette {
        name: "gruvbox".into(),
        bg: bg.into(),
        bg_elev: bg_elev.into(),
        fg: fg.into(),
        fg_dim: fg_dim.into(),
        fg_dimmer: fg_dimmer.into(),
        border: border.into(),
        hint: fg_dim.into(),
        accent: yellow.into(),
        accent_fg: bg.into(), // bg0, the canvas itself, 8.69:1
        accent_soft: orange.into(),
        accent_alt: green.into(),
        warn: orange.into(),
        destructive: red.into(),
        destructive_fg: "#1d2021".into(), // gruvbox dark0_hard, 4.77:1
        ref_link_fg: aqua.into(),
        tag_link_fg: purple.into(),
        md_link_fg: blue.into(),
        bold_fg: fg.into(),
        italic_fg: fg_soft.into(),
        strike_fg: fg_dimmer.into(),
        highlight_bg: yellow.into(),
        highlight_fg: bg.into(),
        code_fg: green.into(),
        todo_open_fg: orange.into(),
        todo_done_fg: green.into(),
        todo_done_body_fg: fg_dimmer.into(),
        property_key_fg: fg_dimmer.into(),
        property_value_fg: fg_soft.into(),
        heading_fg: fg.into(),
        dim_fg: fg_dimmer.into(),
        selected_bullet_bg: yellow.into(),
        selected_bullet_fg: bg.into(),
        cursor_block_bg: fg.into(),
        cursor_block_fg: bg.into(),
        cursor_caret_fg: fg_soft.into(),
        status_normal_bg: yellow.into(),
        status_normal_fg: bg.into(),
        status_insert_bg: green.into(),
        status_insert_fg: bg.into(),
        status_visual_bg: purple.into(),
        status_visual_fg: bg.into(),
        status_message_fg: orange.into(),
        list_selected_bg: border.into(),
        list_selected_fg: fg.into(),
        help_title_fg: yellow.into(),
    }
}

/// Gruvbox Light — the light side of morhetz's retro-groove palette.
/// Warm cream canvas (`#fbf1c7`), dark warm-gray text (`#3c3836`), and
/// the classic muted accents darkened for a light background. Hex
/// values sourced from the canonical gruvbox palette (light variant);
/// the accent and destructive hues use the palette's normal (dark)
/// color set so they clear WCAG AA on the cream canvas.
pub fn gruvbox_light() -> Palette {
    let bg = "#fbf1c7"; // bg0
    let bg_elev = "#ebdbb2"; // bg1
    let border = "#d5c4a1"; // bg2
    let fg = "#3c3836"; // fg0
    let fg_soft = "#504945"; // fg1
    let fg_dim = "#665c54"; // fg3
    let fg_dimmer = "#7c6f64"; // fg4
    let yellow = "#d7992f";
    let orange = "#af3a03"; // normal orange — bright #d65d0e is too dim on bg0
    let green = "#98971a";
    let aqua = "#427b58";
    let blue = "#076678";
    let purple = "#8f3f71";
    let red = "#9d0006"; // normal red — bright #cc241d is too dim on bg0

    Palette {
        name: "gruvbox-light".into(),
        bg: bg.into(),
        bg_elev: bg_elev.into(),
        fg: fg.into(),
        fg_dim: fg_dim.into(),
        fg_dimmer: fg_dimmer.into(),
        border: border.into(),
        hint: fg_dim.into(),
        accent: orange.into(),
        accent_fg: bg.into(), // the canvas itself, 5.40:1
        accent_soft: yellow.into(),
        accent_alt: green.into(),
        warn: orange.into(),
        destructive: red.into(),
        destructive_fg: "#ebdbb2".into(), // gruvbox light bg1, 6.28:1
        ref_link_fg: aqua.into(),
        tag_link_fg: purple.into(),
        md_link_fg: blue.into(),
        bold_fg: fg.into(),
        italic_fg: fg_soft.into(),
        strike_fg: fg_dimmer.into(),
        highlight_bg: yellow.into(),
        highlight_fg: "#1d2021".into(), // gruvbox dark0_hard
        code_fg: green.into(),
        todo_open_fg: orange.into(),
        todo_done_fg: green.into(),
        todo_done_body_fg: fg_dimmer.into(),
        property_key_fg: fg_dimmer.into(),
        property_value_fg: fg_soft.into(),
        heading_fg: fg.into(),
        dim_fg: fg_dimmer.into(),
        selected_bullet_bg: yellow.into(),
        selected_bullet_fg: "#1d2021".into(),
        cursor_block_bg: fg.into(),
        cursor_block_fg: bg.into(),
        cursor_caret_fg: fg_soft.into(),
        status_normal_bg: yellow.into(),
        status_normal_fg: "#1d2021".into(),
        status_insert_bg: green.into(),
        status_insert_fg: "#1d2021".into(),
        status_visual_bg: "#d3869b".into(), // bright purple — normal #8f3f71 is too dim on dark0_hard
        status_visual_fg: "#1d2021".into(),
        status_message_fg: orange.into(),
        list_selected_bg: border.into(),
        list_selected_fg: fg.into(),
        help_title_fg: orange.into(),
    }
}
