from pathlib import Path
import tempfile

import pytest


@pytest.fixture
def campaign_root():
    # Retain local proof artifacts in the explicitly gitignored captures tree.
    captures = Path(__file__).parent / "captures"
    captures.mkdir(exist_ok=True)
    return Path(tempfile.mkdtemp(prefix="proof-", dir=captures))
