use std::{path::Path, sync::Arc};
use pyo3::{prelude::*, types::PyList, create_exception, exceptions::PyException};
use pyo3::{PyErr, PyErrArguments};
use serde_json::Value;
use tokio::{fs, sync::{Mutex, mpsc::{self,  Receiver}}};
use unicom_lib::error::UnicomErrorKind;
use unicom_lib::{node::{message::request::UnicomRequest, utils::pending::PendingController, NodeConfig}, error::UnicomError};
use pythonize::{pythonize, depythonize};
use self::{server::PythonServer, config::PythonConfig, script::PYTHON_EXECUTE};

pub mod script;
mod server;
mod config;

create_exception!(unicom, UnicomPyError, PyException);

create_exception!(unicom, NotFound, PyException);
create_exception!(unicom, ParameterInvalid, PyException);
create_exception!(unicom, InputInvalid, PyException);
create_exception!(unicom, Internal, PyException);
create_exception!(unicom, NotAllowed, PyException);
create_exception!(unicom, MethodNotAllowed, PyException);
create_exception!(unicom, Empty, PyException);

#[pymodule]
fn unicom(py: Python<'_>, m: &PyModule) -> PyResult<()> {
    m.add("UnicomPyError", py.get_type::<UnicomPyError>())?;
    m.add("NotFound", py.get_type::<NotFound>())?;

    Ok(())
}

#[derive(Debug)]
pub enum PythonMessage{
    Request{
        id: u64,
        data: UnicomRequest
    },
    Quit
}

pub struct App{
    path: String,
    api_objects: Mutex<Vec<PyObject>>,
    close_object: Mutex<Option<PyObject>>,
    server: PyObject,
    pub rx: Mutex<Receiver<PythonMessage>>,
    pub pending: Arc<PendingController>,
    
}

impl App{
    pub fn new(path: String) -> App{
        let (tx, rx) = mpsc::channel(64);
        let pending = Arc::new(PendingController::new());
        let server = Python::with_gil(|py| -> PyResult<PyObject>{
            Ok(Py::new(py, PythonServer::new(tx, pending.clone()))?.into_py(py))
        }).unwrap();

        App{
            path,
            api_objects: Mutex::new(Vec::new()),
            close_object: Mutex::new(None),
            rx: Mutex::new(rx),
            pending,
            server,
        }
    }

    pub async fn initialize(&self) -> NodeConfig{
        let path = Path::new(&self.path);
        let code = fs::read_to_string(path.join("app.py")).await.unwrap();

        let p_config = Python::with_gil(|py| -> PyResult<PythonConfig> {

            py.import("sys")?.getattr("path")?
                .downcast::<PyList>()?
                .insert(0, &self.path)?;
            

            Ok(PyModule::from_code(py, &code, "app.py", "")?
                .getattr("config")?
                .call((&self.server,), None)?
                .extract()?)

        }).unwrap();

        let mut api_objects = self.api_objects.lock().await;
        *api_objects = p_config.api_objects;

        let mut close_object = self.close_object.lock().await;
        *close_object = p_config.close_object;

        p_config.config
    }

    pub async fn execute(&self, request: UnicomRequest) -> Result<Vec<u8>, CustomUnicomError>{
        let api_objects = self.api_objects.lock().await;
        let api = api_objects[request.id as usize].clone();
        drop(api_objects);
        let value = match Python::with_gil(|py| -> PyResult<_> {
            let method: &str = request.method.clone().into();
            let fct = api.getattr(py, method)?;
            let parameters = request.parameters.clone();
            pyo3_asyncio::tokio::into_future(PYTHON_EXECUTE.call1(py,(fct, pythonize(py, &parameters)?, &self.server,))?.as_ref(py))
        }){
            Ok(value) => value,
            Err(e) => return Err(e.into()),
        };
        
        let ret = match value.await{
            Ok(ret) => ret,
            Err(e) => return Err(e.into()),
        };
        let ret_value = match Python::with_gil(|py| -> PyResult<Value> {
            Ok(depythonize(ret.as_ref(py))?)
        }){
            Ok(ret_value) => ret_value,
            Err(e) => return Err(e.into()),
        };
        
        match serde_json::to_string(&ret_value){
            Ok(v) => Ok(v.as_bytes().to_vec()),
            Err(e) => {
                let error: UnicomError = e.into();
                Err(error.into())
            },
        }
    }

    pub fn close(&self){
        println!("close app {}", self.path)
    }
}


#[derive(Debug)]
pub struct CustomUnicomError{
    pub error: UnicomError
}

impl From<UnicomError> for CustomUnicomError{
    fn from(e: UnicomError) -> Self{
        CustomUnicomError { error: e }
    }
}

impl Into<UnicomError> for CustomUnicomError{
    fn into(self) -> UnicomError {
        self.error
    }
}

impl From<CustomUnicomError> for PyErr {
    fn from(err: CustomUnicomError) -> PyErr {
        UnicomPyError::new_err(err.error.to_string())
    }
}

impl Into<CustomUnicomError> for PyErr{
    fn into(self) -> CustomUnicomError {
        let (kind, description) = Python::with_gil(|py| -> (UnicomErrorKind, String){
            if self.is_instance_of::<NotFound>(py){
                let data = self.arguments(py);
                return (UnicomErrorKind::NotFound, data.to_string())
            }
            else if self.is_instance_of::<ParameterInvalid>(py){
                let data = self.arguments(py);
                return (UnicomErrorKind::ParameterInvalid, data.to_string())
            }
            else if self.is_instance_of::<InputInvalid>(py){
                let data = self.arguments(py);
                return (UnicomErrorKind::InputInvalid, data.to_string())
            }
            else if self.is_instance_of::<NotAllowed>(py){
                let data = self.arguments(py);
                return (UnicomErrorKind::NotAllowed, data.to_string())
            }
            else if self.is_instance_of::<MethodNotAllowed>(py){
                let data = self.arguments(py);
                return (UnicomErrorKind::MethodNotAllowed, data.to_string())
            }
            else if self.is_instance_of::<Empty>(py){
                let data = self.arguments(py);
                return (UnicomErrorKind::Empty, data.to_string())
            }
            else{
                let trace = Python::with_gil(|py| -> String{
                    let trace = self.traceback(py);
                    if trace.is_some(){
                        return format!("{}", trace.unwrap().format().unwrap())
                    }
                    return format!("no traceback")
                    
                });
                return (UnicomErrorKind::Internal, format!("{} \n {}", self.to_string(), trace))
            }
        });
        CustomUnicomError { 
            error: UnicomError::new(kind, &description)
        }            
    }
}




