use pyo3::prelude::*;

lazy_static! {
    pub static ref PYTHON_SIGNATURE: PyObject = {
        Python::with_gil(|py| -> PyObject {
            let signature = PyModule::from_code(
                py,
                "
import inspect
def signature(fct):
    ret = []
    s = inspect.signature(fct)
    for key in s.parameters.keys():
        if key == 'server':
            continue
        ret.append({
            'name': key,
            'kind': str(s.parameters[key].annotation),
            'mandatory': s.parameters[key].default == s.parameters[key].empty
        })
    return ret",
                "",
                "",
            ).unwrap().getattr("signature").unwrap();

            return signature.into()
        })

    };
}

lazy_static! {
    pub static ref PYTHON_EXECUTE: PyObject = {
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
        if key in s.parameters.keys():
            b.arguments[key] = parameters[key]

    if 'server' in s.parameters.keys():
        b.arguments['server'] = server

    return fct(*b.args, **b.kwargs)",
                "",
                "",
            ).unwrap().getattr("apply_fct").unwrap();

            return apply_fct.into()
        })

    };
}