import subprocess
from pathlib import Path

import pytest

from dvr_forensics.core import Satya, SatyaError


def _binary_available() -> bool:
    try:
        Satya()
        return True
    except SatyaError:
        return False


pytestmark = pytest.mark.skipif(
    not _binary_available(),
    reason="satya binary not built",
)


def test_identify_hikvision():
    s = Satya()
    info = s.identify("../../data/test_images/synthetic_hikvision.img")
    assert info["oem"].lower() == "hikvision"


def test_identify_dahua():
    s = Satya()
    info = s.identify("../../data/test_images/synthetic_dahua.img")
    assert info["oem"].lower() == "dahua"


def test_hash_is_64_hex_chars():
    s = Satya()
    h = s.hash("../../data/test_images/synthetic_hikvision.img")
    assert len(h) == 64
    int(h, 16)  # must parse as hex
