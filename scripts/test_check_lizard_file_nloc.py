"""Tests for the deterministic Lizard XML file-limit check."""

import unittest

from scripts.check_lizard_file_nloc import ReportError, file_nlocs, violations


def report_with_nloc(nloc: str) -> str:
    return f"""<cppncss><measure type="File"><labels>
    <label>Nr.</label><label>NCSS</label><label>CCN</label>
    </labels><item name="src/example.rs"><value>1</value>
    <value>{nloc}</value><value>2</value></item></measure></cppncss>"""


class LizardFileNlocTests(unittest.TestCase):
    def test_exact_limit_passes(self) -> None:
        self.assertEqual(violations(file_nlocs(report_with_nloc("500")), 500), [])

    def test_over_limit_fails(self) -> None:
        self.assertEqual(
            violations(file_nlocs(report_with_nloc("501")), 500),
            [("src/example.rs", 501)],
        )

    def test_malformed_report_fails_closed(self) -> None:
        with self.assertRaises(ReportError):
            file_nlocs("<cppncss><measure")


if __name__ == "__main__":
    unittest.main()
