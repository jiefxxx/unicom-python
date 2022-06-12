use std::{path::Path, sync::Arc};


use pyo3::{prelude::*, types::PyList};

use async_trait::async_trait;
use tokio::{fs, sync::{Mutex, mpsc::{self,  Receiver}}};

use crate::{error::{UnicomError, UnicomErrorKind}, node::python::config::PythonConfig};

use self::{executor::execute, server::PythonServer};

use super::{NodeConnector, message::{request::UnicomRequest, response::UnicomResponse, UnicomMessage}, NodeConfig, utils::pending::PendingController};

mod config;
mod executor;
mod server;


#[derive(Debug)]
pub enum PythonMessage{
    Request{
        id: u64,
        data: UnicomRequest
    },
    Quit
}

pub struct PythonConnector{
    path: &'static Path,
    api_objects: Mutex<Vec<PyObject>>,
    server: PyObject,
    rx: Mutex<Receiver<PythonMessage>>,
    pending: Arc<PendingController>,
    
}

impl PythonConnector{
    pub async fn new(path: &'static str) -> Result<PythonConnector, UnicomError>{
        let path = Path::new(path);
        let (tx, rx) = mpsc::channel(64);
        let pending = Arc::new(PendingController::new());
        let server = Python::with_gil(|py| -> PyResult<PyObject>{
            Ok(Py::new(py, PythonServer{
                tx,
                pending: pending.clone(),
            })?.into_py(py))
        })?;

        Ok(PythonConnector{
            path,
            api_objects: Mutex::new(Vec::new()),
            rx: Mutex::new(rx),
            pending,
            server,
        })
    }
}

#[async_trait]
impl NodeConnector for PythonConnector{
    async fn init(&self) -> Result<NodeConfig, UnicomError>{
        let code = fs::read_to_string(self.path.join("app.py")).await?;

        let p_config = Python::with_gil(|py| -> PyResult<PythonConfig> {

            py.import("sys")?.getattr("path")?
                .downcast::<PyList>()?
                .insert(0, &self.path)?;

            Ok(PyModule::from_code(py, &code, "", "")?
                .getattr("config")?
                .call((&self.server,), None)?
                .extract()?)
        })?;

        let mut api_objects = self.api_objects.lock().await;
        *api_objects = p_config.api_objects;

        Ok(p_config.config)

    }

    async fn request(&self, request: UnicomRequest, _timeout: f32) -> Result<UnicomResponse, UnicomError>{
        let api_objects = self.api_objects.lock().await;
        let api = api_objects[request.id as usize].clone();
        drop(api_objects);
        Ok(UnicomResponse::from_string(execute(&api, &request, &self.server).await?))
    }

    async fn response(&self, request_id: u64, response: UnicomResponse) -> Result<(), UnicomError>{
        self.pending.update(request_id, Ok(response.data)).await
    }

    async fn error(&self, request_id: u64, error: UnicomError) -> Result<(), UnicomError>{
        self.pending.update(request_id, Err(error)).await
    }

    async fn next(&self) -> Result<UnicomMessage, UnicomError>{
        let mut rx = self.rx.lock().await;
        let mess = rx.recv().await;
        if mess.is_none(){
            return Ok(UnicomMessage::Quit)
        }
        let mess = mess.unwrap();
        match mess{
            PythonMessage::Request { id, data } => Ok(UnicomMessage::Request { id, data }),
            PythonMessage::Quit => Ok(UnicomMessage::Quit),
        }
    }

    async fn quit(&self) -> Result<(), UnicomError>{
        todo!();
    }
}

impl From<PyErr> for UnicomError{
    fn from(e: PyErr) -> Self {
        UnicomError::new(UnicomErrorKind::ParseError, &format!("Python error : {:?}",e))
    }
}

