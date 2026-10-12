#!/usr/bin/env python3
"""Prepare the supplied installer artwork; never regenerate the design."""
from pathlib import Path
import subprocess
ROOT = Path(__file__).resolve().parents[1]
subprocess.run(["sips", "--resampleWidth", "768", str(ROOT / "assets/brand/Mira_macOSInstaller.png"), "--out", str(ROOT / "apps/desktop/installer/background.png")], check=True)
