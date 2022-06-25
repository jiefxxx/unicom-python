use std::{path::Path, sync::Arc};
use pyo3::types::PyBytes;
use pyo3::{prelude::*, types::PyList, create_exception, exceptions::PyException};
use pyo3::{PyErr, PyErrArguments};
use serde_json::Value;
use tokio::{fs, sync::{Mutex, mpsc::{self,  Receiver, Sender}}};
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


#[derive(Debug)]
pub enum PythonMessage{
    Request{
        id: u64,
        data: UnicomRequest
    },
    Quit
}

enum PythonReturn{
    Binary(Vec<u8>),
    Value(Value),
}

pub struct App{
    api_objects: Vec<PyObject>,
    run_object: Option<PyObject>,
    close_object: Option<PyObject>,
    server: PyObject,
    pub config: NodeConfig,
    pub rx: Mutex<Receiver<PythonMessage>>,
    pub tx: Sender<PythonMessage>,
    pub pending: Arc<PendingController>,
    
}



impl App{
    pub async fn new(path: String) -> App{

        println!("app path {}", path);
        let (tx, rx) = mpsc::channel(64);
        let pending = Arc::new(PendingController::new());

        let code = fs::read_to_string(Path::new(&path).join("app.py")).await.expect("uneable to read app.py");
        let (config, run, close) = Python::with_gil(|py| -> PyResult<_> {

            py.import("sys")?.getattr("path")?
                .downcast::<PyList>()?
                .insert(0, &path)?;
            
                let module = PyModule::from_code(py, &code, "app.py", "")?;

                let config = module.getattr("config").expect("config fct error").into_py(py);

                let run = match module.getattr("run"){
                    Ok(run) => Some(run.into_py(py)),
                    Err(_) => None,
                };
                let close = match module.getattr("close"){
                    Ok(close) => Some(close.into_py(py)),
                    Err(_) => None,
                };
                
                Ok((config, run, close))

        }).expect("python import error");

        let server = Python::with_gil(|py| -> PyResult<PyObject>{
            Ok(Py::new(py, PythonServer::new(tx.clone(), pending.clone()))?.into_py(py))
        }).expect("init server object error");

        let ret = Python::with_gil(|py| -> PyResult<_> {

            Ok(pyo3_asyncio::tokio::into_future(config.call1(py, (&server,))?.into_ref(py))?)

        }).expect("call config failed").await.expect("config error");

        let p_config = Python::with_gil(|py| -> PyResult<PythonConfig> {

            Ok(ret.extract(py)?)

        }).expect("config extract error");

        App{
            api_objects: p_config.api_objects,
            run_object: run,
            close_object: close,
            server,
            config: p_config.config,
            rx: Mutex::new(rx),
            pending,
            tx,
        }
    }

    pub fn runnable(&self) -> bool{
        if self.run_object.is_none(){
            false
        }else{
            true
        }
    }

    pub async fn run(&self){
        if self.run_object.is_none(){
            return
        }
        if let Err(e) = Python::with_gil(|py| -> PyResult<_> {

            Ok(pyo3_asyncio::tokio::into_future(self.run_object.as_ref().unwrap().call1(py, (&self.server,))?.into_ref(py))?)

        }).expect("call run failed").await{
            let error: CustomUnicomError = e.into();
            println!("run failed : {}", error.error.description);
        }
    }

    pub async fn execute(&self, request: UnicomRequest) -> Result<Vec<u8>, CustomUnicomError>{
        let api = match self.api_objects.get(request.id as usize){
            Some(api) => api,
            None => return Err(UnicomError::new(UnicomErrorKind::NotFound, &format!("api_id not found {}", request.id)).into()),
        };

        let ret = match Python::with_gil(|py| -> PyResult<_> {
            let method: &str = request.method.clone().into();
            let fct = api.getattr(py, method)?;
            pyo3_asyncio::tokio::into_future(PYTHON_EXECUTE.call1(py,(fct, pythonize(py, &request.parameters)?, &self.server,))?.as_ref(py))
        }){
            Ok(value) => {
                match value.await{
                    Ok(ret) => ret,
                    Err(e) => return Err(e.into()),
                }
            },
            Err(e) => return Err(e.into()),
        };

        match Python::with_gil(|py| -> PyResult<PythonReturn> {
            match ret.cast_as::<PyBytes>(py){
                Ok(data) => Ok(PythonReturn::Binary(data.as_bytes().to_vec())),
                Err(_) => Ok(PythonReturn::Value(depythonize(ret.as_ref(py))?)),
            }
        }){
            Ok(p_return) => match p_return {
                PythonReturn::Binary(bin) => Ok(bin),
                PythonReturn::Value(v) => {
                    match serde_json::to_string(&v){
                        Ok(v) => Ok(v.as_bytes().to_vec()),
                        Err(e) => {
                            let error: UnicomError = e.into();
                            Err(error.into())
                        },
                    }
                },
            },
            Err(e) => Err(e.into()),
        }
        
    }

    pub async fn close(&self){
        self.tx.send(PythonMessage::Quit).await.expect("send quit error");
        if self.close_object.is_none(){
            return
        }
        Python::with_gil(|py| -> PyResult<_> {
            pyo3_asyncio::tokio::into_future(self.close_object.as_ref().unwrap().call1(py,(&self.server, ))?.as_ref(py))
        }).expect("call close failed").await.expect("error on close");
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




