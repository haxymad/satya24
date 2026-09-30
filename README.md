# SATYA — Multi-Vendor DVR/NVR Forensic Analysis Tool

A vendor-agnostic forensic platform for standardized acquisition, recovery,
and analysis of surveillance evidence across major DVR/NVR OEMs.

## Problem

Digital/Network Video Recorders (DVR/NVRs) from different manufacturers
(Hikvision, Dahua, CP Plus, Honeywell, Uniview, TP-Link, Godrej, Matrix)
use proprietary storage formats, file systems, and metadata structures.
Investigators must switch between vendor-specific tools, resulting in
inconsistent results, timestamp synchronization issues, and difficulty
maintaining evidence integrity.

## Solution

SATYA is a unified forensic platform that:

1. **Identifies** the DVR/NVR OEM automatically from a raw disk image
2. **Parses** proprietary file systems across 8 major vendors
3. **Recovers** deleted video frames from unallocated space
4. **Extracts** timestamps from multiple sources (frame headers, OCR, NTP logs)
5. **Fuses** them into per-frame timestamps with **95% confidence intervals**
   using the *Temporal Trust Engine* (novel contribution)
6. **Signs** every artifact with hybrid Ed25519 + ML-DSA signatures
7. **Generates** court-ready PDF reports with uncertainty envelopes

## Novelty

The **Temporal Trust Engine** addresses a gap in current tools: existing
software reports a single timestamp per frame without quantifying how much
that timestamp can be trusted. Our engine:

- Collapses same-clock claims (frame header + OCR + device log all share
  ONE DVR clock — agreement between them does not prove clock correctness)
- Fuses independent anchors (NTP sync logs, ENF hum, solar position)
  using Bayesian Gaussian inference
- Emits a posterior distribution per frame, not a point estimate

This directly supports evidence admissibility requirements that ask for
known error rates in forensic methods.

## Architecture
