use pyo3::{prelude::*, exceptions::asyncio};
use pythonize::pythonize;
use crate::{node::message::request::{UnicomRequest, self}, error::UnicomError};

lazy_static! {
    static ref PYTHON_EXECUTE: PyObject = {
        Python::with_gil(|py| -> PyObject {
            let apply_fct = PyModule::from_code(
                py,
                "
import asyncio
import inspect
def apply_fct(fct, parameters, server):
    s = inspect.signature(fct)
    b = s.bind_partial()
    b.apply_defaults()
    for key in parameters.keys():
        if key == 'server':
            b.arguments[key] = server
        elif key in s.parameters.keys():
            b.arguments[key] = parameters[key]
    return fct(*b.args, **b.kwargs)",
                "",
                "",
            ).unwrap().getattr("apply_fct").unwrap();

            return apply_fct.into()
        })

    };
}


pub async fn execute(api: &PyObject, request: &UnicomRequest, server: &PyObject) -> Result<String, UnicomError>{
    pyo3::prepare_freethreaded_python();
    Ok(Python::with_gil(|py| -> PyResult<String> {
        let method: &str = request.method.clone().into();
        let fct = api.getattr(py, method)?;
        let parameters = request.parameters.clone();
        let ret = PYTHON_EXECUTE.call1(py,(fct, pythonize(py, &parameters)?, server,))?;
        ret.extract(py)
    })?)
}