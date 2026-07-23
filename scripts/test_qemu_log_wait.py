#!/usr/bin/env python3
import importlib.util
import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).with_name("qemu_log_wait.py")
SPEC = importlib.util.spec_from_file_location("qemu_log_wait", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class LogEvaluationTests(unittest.TestCase):
    def test_requires_every_success_condition(self):
        state, _ = MODULE.evaluate_log("READY\n", ["READY", "DONE"], [], [], [])
        self.assertEqual(state, "pending")

    def test_exact_line_does_not_accept_substring(self):
        state, _ = MODULE.evaluate_log("prefix DONE suffix\n", ["DONE"], [], [], [])
        self.assertEqual(state, "pending")

    def test_success_regex_is_full_line(self):
        regex = r"BUILDSTORM_COMPILE mode=multi ok=true .*"
        state, _ = MODULE.evaluate_log(
            "BUILDSTORM_COMPILE mode=multi ok=true elapsed_s=3.2 cores=8 bytes=600000 arch=riscv64\n",
            [],
            [regex],
            [],
            [],
        )
        self.assertEqual(state, "success")

    def test_failure_wins_over_success(self):
        state, matched = MODULE.evaluate_log(
            "DONE\nBUILDSTORM_MINIBUILD fail\n",
            ["DONE"],
            [],
            ["BUILDSTORM_MINIBUILD fail"],
            [],
        )
        self.assertEqual((state, matched), ("failure", "BUILDSTORM_MINIBUILD fail"))


if __name__ == "__main__":
    unittest.main()
