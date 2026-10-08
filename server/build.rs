// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
use std::{env, fs, path::PathBuf};
use two_face::{
    re_exports::syntect::html::{ClassStyle, css_for_theme_with_class_style},
    theme::{EmbeddedThemeName, extra},
};

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let css = css_for_theme_with_class_style(
        &extra()[EmbeddedThemeName::CatppuccinMocha],
        ClassStyle::SpacedPrefixed { prefix: "syn-" },
    )
    .unwrap();
    fs::write(out.join("theme.css"), css).unwrap();
    fs::write(
        out.join("syntax-notices.txt"),
        format!("{:#?}", two_face::acknowledgement::listing()),
    )
    .unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}
