"""Architecture guard characterization, including Rust lexical edge cases."""

import unittest

from check_architecture import violations


class ArchitectureTests(unittest.TestCase):
    def check(self, text, expected, path="src/inbound/gui/app.rs"):
        self.assertEqual(bool(list(violations(path, text))), expected)

    def test_nested_groups_aliases_and_multiline(self):
        self.check(
            "use crate::{domain::image::ImageBuffer,\n core::{self as c, ScanError}};",
            True,
        )
        self.check(
            "use crate::{infrastructure::{media as m}, domain::image::*};", False
        )
        self.check("use crate :: r#core :: ScanError as Error;", True)
        self.check(
            "use crate::{infrastructure::{media as m}};", True, "src/domain/x.rs"
        )

    def test_crate_alias_cannot_hide_grouped_dependency(self):
        self.check("use crate as root; use root::{core::ScanError};", True)
        self.check("use crate::{self as root}; root::core::ScanError;", True)

    def test_strings_comments_and_lifetimes(self):
        self.check('let s = "crate::core::X"; // crate::core::X\n', False)
        self.check('let s = br##"use crate::{core::X};"##;', False)
        self.check("/* outer /* crate::core::X */ still crate::core::X */", False)
        self.check("fn x<'a>(x: &'a crate::core::X) {}", True)
        self.check("let x = '\"'; use crate::{core::X};", True)

    def test_existing_rules(self):
        self.check("let x = crate::workflows::x();", True, "src/domain/x.rs")
        self.check("use crate::infrastructure::x;", True, "src/workflows/x.rs")
        self.check('#[path = "other.rs"] mod x;', True, "src/domain/x.rs")
        self.check("pub mod infrastructure;", True, "src/lib.rs")
        self.check("pub use crate::domain::image::*;", False, "src/core.rs")
        self.check("pub fn x() {}", True, "src/core.rs")
