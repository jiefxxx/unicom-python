use std::collections::HashMap;

use pyo3::{prelude::*, types::{PyDict, PyList}};
use pythonize::depythonize;

use crate::node::{api::{Parameter, ApiMethod}, NodeConfig, endpoint::{EndPointKind, ApiConfig}};



#[derive(Debug, Clone)]
#[pyclass]
pub struct PythonConfig{
    pub config: NodeConfig,
    pub api_objects: Vec<PyObject>,
}

impl PythonConfig{
    pub fn new(name: &str) -> PythonConfig{
        PythonConfig { 
            config: NodeConfig::new(name), 
            api_objects: Vec::new(),
        }
    }
}

#[pymethods]
impl PythonConfig{
    pub fn add_template(&mut self, file: &str, path: &str){
        self.config.add_template(file, path);
    }

    pub fn add_api(&mut self, name: String, object: PyObject) -> PyResult<String>{
        let mut methodes = Vec::new();
        let list_methodes = vec!["GET", "POST", "PUT", "DELETE"];
        Python::with_gil(|py| -> PyResult<()>{
            for s_methode in list_methodes{
                if let Ok(methode) = object.getattr(py, s_methode){
                    let data = PYTHON_SIGNATURE.call1(py, (methode,))?;
                    let list: &PyList = data.extract(py)?;
                    let mut parameters = Vec::new();
                    for dict in list{
                        let dict: &PyDict = dict.extract().unwrap();
                        let p_name = dict.get_item("name").unwrap().extract()?;
                        let p_kind: &str = dict.get_item("kind").unwrap().extract()?;
                        let p_mandatory = dict.get_item("mandatory").unwrap().extract()?;
                        parameters.push(Parameter::new(p_name, p_kind.into(), p_mandatory));
                    }
                    methodes.push(ApiMethod::new(s_methode.into(), parameters))
    
                }
            }

            Ok(())
            
        })?;

        let id = self.api_objects.len() as u64;
        self.config.add_api(id, &name, methodes);
        self.api_objects.push(object);

        Ok(name)
    }

    pub fn add_static(&mut self, regex: String, path: String){
        self.config.add_endpoint(&regex, EndPointKind::Static { path });
    }

    pub fn add_dynamic(&mut self, regex: String, api: String){
        self.config.add_endpoint(&regex, EndPointKind::Dynamic { api });
    }

    pub fn add_rest(&mut self, regex: String, api: String){
        self.config.add_endpoint(&regex, EndPointKind::Rest { api });
    }

    pub fn add_view(&mut self, regex: String, template: String, dict: &PyDict) -> PyResult<()>{
        let apis: HashMap<String, ApiConfig> = depythonize(dict)?;
        self.config.add_endpoint(&regex, EndPointKind::View { apis, template });
        Ok(())
    }
}