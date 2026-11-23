"""Keep every test's case-check log out of the real state folder."""
import pytest


@pytest.fixture(autouse=True)
def isolated_state_folder(tmp_path_factory, monkeypatch):
    monkeypatch.setenv("XDG_STATE_HOME", str(tmp_path_factory.mktemp("state")))
