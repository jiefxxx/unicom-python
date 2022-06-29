use std::{sync::Arc, collections::HashMap};


use pyo3::{prelude::*, types::PyDict, exceptions};

use pythonize::{pythonize, depythonize};
use serde_json::{Map, Value};
use tokio::sync::mpsc::{Sender, self};
use unicom_lib::{node::{utils::pending::PendingController, message::request::UnicomRequest}, error::UnicomError};


use super::{PythonMessage, config::PythonConfig, CustomUnicomError, NotFound, ParameterInvalid, InputInvalid, Internal, NotAllowed, MethodNotAllowed, Empty};



#[pyclass]
pub struct PythonServer{
    tx: Sender<PythonMessage>,
    pending: Arc<PendingController>,
    user_data: HashMap<String, PyObject>,
    background_worker: HashMap<String, Sender<PyObject>>,

    #[pyo3(get)]
    config: PythonConfig

}

impl PythonServer{
    pub fn new(tx: Sender<PythonMessage>, pending: Arc<PendingController>) -> PythonServer{
        PythonServer{
            tx,
            pending,
            user_data: HashMap::new(),
            background_worker: HashMap::new(),
            config: PythonConfig::new(),
        }
    }
}

#[pymethods]
impl PythonServer{

    pub fn error_not_found(&self, message: String) -> PyErr{
        NotFound::new_err(message)
    }

    pub fn error_parameter_invalid(&self, message: String) -> PyErr{
        ParameterInvalid::new_err(message)
    }

    pub fn error_input_invalid(&self, message: String) -> PyErr{
        InputInvalid::new_err(message)
    }

    pub fn error_internal(&self, message: String) -> PyErr{
        Internal::new_err(message)
    }

    pub fn error_not_allowed(&self, message: String) -> PyErr{
        NotAllowed::new_err(message)
    }

    pub fn error_method_not_allowed(&self, message: String) -> PyErr{
        MethodNotAllowed::new_err(message)
    }

    pub fn error_empty(&self, message: String) -> PyErr{
        Empty::new_err(message)
    }

    #[args(kwargs="**")]
    fn request<'p>(&self, py: Python<'p>, node: String, api: String, method: String, kwargs: Option<&PyDict>) -> PyResult<&'p PyAny> {
        let tx = self.tx.clone();
        let pending = self.pending.clone();
        let mut parameters = Map::new();
        if let Some(kwargs) = kwargs{
            parameters = depythonize(kwargs)?;
        }

        pyo3_asyncio::tokio::future_into_py_with_locals(
            py, 
            pyo3_asyncio::tokio::get_current_locals(py)?,
            async move {
                let mut request = UnicomRequest::new();
                request.node_name = node;
                request.method = method.into();
                request.name = api;
                request.parameters = parameters;

                let (id, notify) = pending.create().await;

                tx.send(PythonMessage::Request{
                    id,
                    data: request,
                }).await.unwrap();

                notify.notified().await;

                let data = match pending.get(id).await{
                    Ok(data) => data,
                    Err(e) => {
                        let custom : CustomUnicomError = e.into();
                        return Err(custom.into())
                    },
                };
                let st = String::from_utf8(data)?;
                //println!("receive raw: {}", st);

                let value: Value = match serde_json::from_str(&st){
                    Ok(v) => v,
                    Err(e) => {
                        let error : UnicomError = e.into();
                        let custom : CustomUnicomError = error.into();
                        return Err(custom.into())
                    },
                };
                

                Python::with_gil(|py: Python<'_>| -> PyResult<Py<PyAny>> {
                    Ok(pythonize(py, &value)?)
            })
        })

    }

    pub fn create_user_data(&mut self, name: String, py_object: PyObject){
        self.user_data.insert(name, py_object);
    }

    pub fn get_user_data(&self, name: String) -> Option<&PyObject>{
        self.user_data.get(&name)
    }

    pub fn create_bg_worker(mut self_: PyRefMut<Self>, py: Python, name: String, callable: PyObject) -> PyResult<()>{
        let (tx, mut rx) = mpsc::channel(64);
        self_.background_worker.insert(name, tx);
        let test = self_.into_py(py);
        pyo3_asyncio::tokio::future_into_py_with_locals(
            py,
            pyo3_asyncio::tokio::get_current_locals(py)?,
            async move { 
                loop{
                    let py_object = rx.recv().await;
                    if py_object.is_none(){
                        break
                    }

                    let value = Python::with_gil(|py| -> PyResult<_> {
                        
                        pyo3_asyncio::tokio::into_future(callable.call1(py,(test.clone(), &py_object.unwrap(),))?.as_ref(py))
                    })?;

                    value.await?;
                }
                Ok(())
             }
        )?;
        Ok(())
    }

    pub fn send_bg_worker<'p>(&'p mut self, py: Python<'p>, name: String, object: PyObject) -> PyResult<&'p PyAny>{
        let data = self.background_worker.get(&name);
        if data.is_none(){
            return Err(exceptions::PyTypeError::new_err("no background worker found"))
        }
        let data = data.unwrap().clone();
        pyo3_asyncio::tokio::future_into_py_with_locals(
            py, 
            pyo3_asyncio::tokio::get_current_locals(py)?,
            async move {
                data.send(object).await.unwrap();
                Ok(())
            }
        )
    }

    pub fn send_bg_worker_thread_safe(& mut self, name: String, object: PyObject) -> PyResult<()>{
        let data = self.background_worker.get(&name);
        if data.is_none(){
            return Err(exceptions::PyTypeError::new_err("no background worker found"))
        }
        let data = data.unwrap().clone();

        futures::executor::block_on(data.send(object)).unwrap();

        Ok(())
    }

}
