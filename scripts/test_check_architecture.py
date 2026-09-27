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
            "use crate::{infrastructure::{media as m}, domain::image::*};",
            False,
            "src/workflows/x.rs",
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

    def test_domain_cannot_import_outer_layers(self):
        self.check(
            "use crate::infrastructure::media::load_image;",
            True,
            "src/domain/x.rs",
        )
        self.check("use crate::workflows::capture::batch::Foo;", True, "src/domain/x.rs")
        self.check("use crate::inbound::cli::run;", True, "src/domain/x.rs")

    def test_workflows_can_import_infrastructure_but_not_inbound(self):
        self.check(
            "use crate::infrastructure::acquisition::resolve_device_id;",
            False,
            "src/workflows/x.rs",
        )
        self.check("use crate::inbound::cli::run;", True, "src/workflows/x.rs")

    def test_infrastructure_cannot_import_workflows_or_inbound(self):
        self.check(
            "use crate::workflows::capture::batch::Foo;",
            True,
            "src/infrastructure/x.rs",
        )
        self.check("use crate::inbound::cli::run;", True, "src/infrastructure/x.rs")

    def test_inbound_cannot_import_facades(self):
        self.check("use crate::scan::run_scan_to_file;", True, "src/inbound/x.rs")
        self.check(
            "use crate::infrastructure::media::load_image;", False, "src/inbound/x.rs"
        )

    def test_inbound_infrastructure_allowlist_admits_leaf_helpers_only(self):
        self.check(
            "use crate::infrastructure::config::json::load_config;",
            False,
            "src/inbound/x.rs",
        )
        self.check(
            "use crate::infrastructure::acquisition::open_device;",
            True,
            "src/inbound/x.rs",
        )

    def test_pipeline_facade_module_is_covered_by_the_facade_rule(self):
        self.check("pub fn x() {}", True, "src/pipeline/mod.rs")
        self.check("pub use crate::domain::processing::*;", False, "src/pipeline/mod.rs")

    def test_existing_rules(self):
        self.check("let x = crate::workflows::x();", True, "src/domain/x.rs")
        self.check("use crate::inbound::x;", True, "src/workflows/x.rs")
        self.check('#[path = "other.rs"] mod x;', True, "src/domain/x.rs")
        self.check("pub mod infrastructure;", True, "src/lib.rs")
        self.check("pub use crate::domain::image::*;", False, "src/core.rs")
        self.check("pub fn x() {}", True, "src/core.rs")
