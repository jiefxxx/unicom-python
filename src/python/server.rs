use std::sync::Arc;


use pyo3::{prelude::*, types::PyDict};

use pythonize::depythonize;
use serde_json::Map;
use tokio::sync::mpsc::Sender;

use crate::node::{python::config::PythonConfig, utils::pending::PendingController, message::request::UnicomRequest};

use super::PythonMessage;



#[pyclass]
pub struct PythonServer{
    pub tx: Sender<PythonMessage>,
    pub pending: Arc<PendingController>,
    
}

#[pymethods]
impl PythonServer{

    pub fn pyconfig(&self) -> PythonConfig{
        PythonConfig::new("test")
    }

    #[args(kwargs="**")]
    fn request<'p>(&self, py: Python<'p>, node: String, api: String, kwargs: Option<&PyDict>) -> PyResult<&'p PyAny> {
        let tx = self.tx.clone();
        let pending = self.pending.clone();
        let mut parameters = Map::new();
        if let Some(kwargs) = kwargs{
            parameters = depythonize(kwargs).unwrap()
        }

        pyo3_asyncio::tokio::future_into_py_with_locals(
            py, 
            pyo3_asyncio::tokio::get_current_locals(py)?,
            async move {
                let mut request = UnicomRequest::new();

                request.node_name = node;
                request.name = api;
                request.parameters = parameters;

                let (id, notify) = pending.create().await;

                tx.send(PythonMessage::Request{
                    id,
                    data: request,
                }).await.unwrap();

                notify.notified().await;
                

                Ok()
        })
    }
}
