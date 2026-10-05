"""Stdlib-only tagged unittest runner, copied into the trial image for scoring.

Never includes exception messages, source, tracebacks or captured output in its
JSON. This file is also the case tests' `requirement` decorator import.
"""
import argparse
import contextlib
import io
import json
from pathlib import Path
import sys
import unittest

# Running as a script must expose the same decorator module to discovered tests.
if __name__ == "__main__":
    sys.modules["case_runtime"] = sys.modules[__name__]


def requirement(requirement_id):
    def decorate(test):
        test.requirement_id = requirement_id
        return test
    return decorate


def _flatten(suite):
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            yield from _flatten(item)
        else:
            yield item


class TaggedResult(unittest.TestResult):
    def __init__(self, records):
        super().__init__()
        self.records = records
        self.collection_error = False

    def startTest(self, test):
        super().startTest(test)
        if test.id() in self.records:
            self.records[test.id()]["status"] = "passed"
        else:
            self.collection_error = True

    def _mark(self, test, status):
        record = self.records.get(test.id())
        if record is None:
            # Includes errors/skips in module and class setup/teardown.
            self.collection_error = True
        else:
            record["status"] = status

    def addError(self, test, err):
        self._mark(test, "error")

    def addFailure(self, test, err):
        self._mark(test, "failed")

    def addSkip(self, test, reason):
        self._mark(test, "skipped")

    def addExpectedFailure(self, test, err):
        self._mark(test, "failed")

    def addUnexpectedSuccess(self, test):
        self._mark(test, "failed")

    def addSubTest(self, test, subtest, err):
        if err is not None:
            self._mark(test, "failed" if issubclass(err[0], test.failureException) else "error")


def run_tests(tests_directory):
    sys.path.insert(0, str(Path.cwd()))
    records = {}
    collection_error = False
    # Test output stays in memory, never in receipts or container console logs.
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
        suite = unittest.defaultTestLoader.discover(str(tests_directory))
        for test in _flatten(suite):
            method = getattr(test, getattr(test, "_testMethodName", ""), None)
            tag = getattr(method, "requirement_id", None)
            if not tag or test.id() in records:
                collection_error = True
            else:
                records[test.id()] = {"test_id": test.id(), "requirement_id": tag, "status": "not_run"}
        result = TaggedResult(records)
        suite.run(result)
    return {"tests": list(records.values()), "collection_error": collection_error or result.collection_error}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tests", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = run_tests(args.tests)
    args.output.write_text(json.dumps(report, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
