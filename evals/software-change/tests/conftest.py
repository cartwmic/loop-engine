"""Fixtures every software-change test gets: a private state folder and no cached daemon chip."""
import pytest

from scevals import image


@pytest.fixture(autouse=True)
def isolated_state_folder(tmp_path_factory, monkeypatch):
    """Keep every test's case-check log out of the real state folder."""
    monkeypatch.setenv("XDG_STATE_HOME", str(tmp_path_factory.mktemp("state")))


@pytest.fixture(autouse=True)
def fresh_daemon_arch():
    """The harness reads the daemon's chip once and caches it. A test that fakes the chip must not leak it to the next,
    and a test that forgets to fake it must reach no real daemon, so nothing is cached between tests."""
    image.daemon_arch.cache_clear()
    yield
    image.daemon_arch.cache_clear()
