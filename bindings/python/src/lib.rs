//! Python bindings for `dvb-si` — read-only DVB SI/PSI parsing for scripting.
//!
//! `parse_section(bytes) -> dict` and a `Demux` class wrapping `SiDemux`
//! (`feed(packet) -> list[dict]`), plus `T2miPump` for T2-MI. Parsed structures
//! are converted Rust → `serde_json::Value` → Python objects, so the binding is
//! read-only by design (mirrors the crate's Serialize-only serde posture).

use dvb_si::demux::SiDemux;
use dvb_si::tables::AnyTableSection;
use dvb_t2mi::pump::T2miPump;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde::Serialize;

/// Serialize any `serde::Serialize` value to a Python object via `serde_json`.
fn to_py<T: Serialize>(py: Python<'_>, value: &T) -> PyResult<PyObject> {
    let json = serde_json::to_value(value).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let obj = pythonize::pythonize(py, &json).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(obj.unbind())
}

/// Parse a single SI/PSI section (table) from raw bytes into a Python dict.
#[pyfunction]
fn parse_section(py: Python<'_>, data: &[u8]) -> PyResult<PyObject> {
    let section =
        AnyTableSection::parse(data).map_err(|e| PyValueError::new_err(e.to_string()))?;
    to_py(py, &section)
}

/// PID-filtered, version-gated SI section demux. Feed 188-byte TS packets.
#[pyclass]
struct Demux {
    inner: SiDemux,
}

#[pymethods]
impl Demux {
    #[new]
    fn new() -> Self {
        Demux {
            inner: SiDemux::builder().build(),
        }
    }

    /// Feed one aligned 188-byte TS packet; return the dicts of any newly
    /// completed, changed sections (empty list if none).
    fn feed(&mut self, py: Python<'_>, packet: &[u8]) -> PyResult<Vec<PyObject>> {
        let mut out = Vec::new();
        for event in self.inner.feed(packet) {
            if let Ok(section) = event.table_section() {
                out.push(to_py(py, &section)?);
            }
        }
        Ok(out)
    }
}

/// T2-MI pump over a fixed PID. Feed 188-byte TS packets; get payload dicts.
#[pyclass]
struct T2miDemux {
    inner: T2miPump,
}

#[pymethods]
impl T2miDemux {
    #[new]
    fn new(pid: u16) -> Self {
        T2miDemux {
            inner: T2miPump::new(pid),
        }
    }

    /// Feed one TS packet; return dicts of the decoded T2-MI payloads.
    fn feed(&mut self, py: Python<'_>, packet: &[u8]) -> PyResult<Vec<PyObject>> {
        let mut out = Vec::new();
        for event in self.inner.feed_ts(packet) {
            if let Ok(payload) = event.payload() {
                out.push(to_py(py, &payload)?);
            }
        }
        Ok(out)
    }
}

/// The `dvb_si_py` extension module.
#[pymodule]
fn dvb_si_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse_section, m)?)?;
    m.add_class::<Demux>()?;
    m.add_class::<T2miDemux>()?;
    Ok(())
}
