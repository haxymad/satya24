//! Python bridge — exposes the Rust core to the Python ML layer.
//!
//! Python is used ONLY for ML inference (YOLOv8, ArcFace, XAI).
//! All parsing, trust fusion, custody, and reporting live in Rust.
use pyo3::PyAny;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

#[pyfunction]
fn identify_oem(image_path: &str) -> PyResult<String> {
    let data = std::fs::read(image_path)?;
    Ok(satya_parsers::identify_device(&data)
    .map(|fp| fp.oem.as_str().to_string())
    .unwrap_or_else(|| "unknown".into()))
}

#[pyfunction]
fn enumerate_frames(py: Python<'_>, image_path: &str) -> PyResult<Py<PyAny>> {    let data = std::fs::read(image_path)?;
    let fp = satya_parsers::identify_device(&data)
    .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("unknown OEM"))?;
    let frames = satya_parsers::enumerate_frames(&data, fp.oem)
    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

    let out = PyList::empty(py);
    for f in frames {
        let d = PyDict::new(py);
        d.set_item("offset", f.offset)?;
        d.set_item("length", f.length)?;
        d.set_item("codec", format!("{:?}", f.codec))?;
        d.set_item("recovery_source", format!("{:?}", f.recovery_source))?;
        d.set_item("recovery_confidence", f.recovery_confidence)?;
        out.append(d)?;
    }
    Ok(out.into())
}

#[pyfunction]
fn fuse_timestamps(py: Python<'_>, claims_json: &str) -> PyResult<Py<PyAny>> {
    use satya_trust::{fuse_with_diagnostics, to_claim, Source};
    let raw: Vec<serde_json::Value> = serde_json::from_str(claims_json)
    .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;    let mut claims = Vec::new();
    for c in raw {
        let src = match c["source"].as_str().unwrap_or("") {
            "ntp_sync_log" => Source::NtpSyncLog,
            "enf" => Source::Enf,
            "on_screen_ocr" => Source::OnScreenOcr,
            "device_log" => Source::DeviceLog,
            "solar" => Source::Solar,
            _ => Source::FrameHeader,
        };
        claims.push(to_claim(
            c["mean_s"].as_f64().unwrap_or(0.0),
                             src,
                             c["confidence"].as_f64().unwrap_or(0.5),
        ));
    }
    let ft = fuse_with_diagnostics(&claims);
    let d = PyDict::new(py);
    d.set_item("mean_s", ft.mean_s)?;
    d.set_item("sigma_s", ft.sigma_s)?;
    d.set_item("ci_lo_95_s", ft.ci_lo_95_s)?;
    d.set_item("ci_hi_95_s", ft.ci_hi_95_s)?;
    d.set_item("collapsed", ft.collapsed)?;
    Ok(d.into())
}

#[pymodule]
fn _satya_ml(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(identify_oem, m)?)?;
    m.add_function(wrap_pyfunction!(enumerate_frames, m)?)?;
    m.add_function(wrap_pyfunction!(fuse_timestamps, m)?)?;
    Ok(())
}
