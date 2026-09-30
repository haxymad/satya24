use pyo3::prelude::*;
use pyo3::types::PyList;
use satya_core::*;

#[pyfunction]
fn identify_oem(image_path: &str) -> PyResult<String> {
    let data = std::fs::read(image_path)?;
    Ok(satya_parsers::identify_device(&data)
    .map(|fp| fp.oem.as_str().to_string())
    .unwrap_or_else(|| "unknown".into()))
}

#[pyfunction]
fn enumerate_frames(py: Python<'_>, image_path: &str) -> PyResult<PyObject> {
    let data = std::fs::read(image_path)?;
    let fp = satya_parsers::identify_device(&data)
    .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("unknown OEM"))?;
    let frames = satya_parsers::enumerate_frames(&data, fp.oem)
    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

    let out = PyList::empty(py);
    for f in frames {
        let claims: Vec<(i64, f32, String)> = f.claims.iter().map(|c| {
            (c.claimed_utc.timestamp(), c.confidence,
             format!("{:?}", c.source))
        }).collect();
        out.append((
            f.offset,
            f.length,
            format!("{:?}", f.codec),
                format!("{:?}", f.recovery_source),
                    f.recovery_confidence,
                    claims,
        ))?;
    }
    Ok(out.into())
}

#[pyfunction]
fn sha256_file(image_path: &str) -> PyResult<String> {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(image_path)?;
    let mut h = Sha256::new();
    h.update(&data);
    Ok(hex::encode(h.finalize()))
}

#[pymodule]
fn _satya_core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(identify_oem, m)?)?;
    m.add_function(wrap_pyfunction!(enumerate_frames, m)?)?;
    m.add_function(wrap_pyfunction!(sha256_file, m)?)?;
    Ok(())
}
