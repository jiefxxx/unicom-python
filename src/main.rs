#[macro_use]
extern crate lazy_static;

use std::{sync::Arc, env};

use app::{App, PythonMessage};
use pyo3::prelude::*;
use tokio::{net::UnixStream, sync::{Mutex, Notify}, signal};

use unicom_lib::arch::unix::{write_init, read_message, UnixMessage, write_message};

mod app;

#[pyo3_asyncio::tokio::main(flavor = "multi_thread", worker_threads = 5)]
async fn main() -> PyResult<()> {
    let args: Vec<String> = env::args().collect();
    let app = Arc::new(App::new(args[1].clone()));
    let close_notify = Arc::new(Notify::new());

    let n_config = app.initialize().await;
    
    let stream = UnixStream::connect("/var/unicom/test3.stream").await.unwrap();
    let (mut reader,mut writer) = stream.into_split();
    write_init(&mut writer, &n_config).await.unwrap();
    let writer = Arc::new(Mutex::new(writer));
    let task_exchange;
    {
        let writer = writer.clone();
        let app = app.clone();
        task_exchange = tokio::spawn(async move {
            loop{
                let mut rx = app.rx.lock().await;
                let mess = rx.recv().await;
                if mess.is_none(){
                    break
                }
                let mess = mess.unwrap();
                match mess{
                    PythonMessage::Request { id, data } => {
                        write_message(&mut *writer.lock().await, UnixMessage::Request { id, data }).await.unwrap();
                    },
                    PythonMessage::Quit => {
                        write_message(&mut *writer.lock().await, UnixMessage::Quit).await.unwrap();
                    },
                }
            }
        });
    }

    Python::with_gil(|py| -> PyResult<()> {
        let app = app.clone();
        let close_notify = close_notify.clone();
        pyo3_asyncio::tokio::future_into_py_with_locals(
            py,
            pyo3_asyncio::tokio::get_current_locals(py)?,
            async move { 
                loop {
                    let mess = match read_message(&mut reader).await {
                        Ok(mess) => mess,
                        Err(e) => {
                            println!("error read message {:?}",e);
                            close_notify.notify_one();
                            return Ok(())
                        },
                    };
                    match mess {
                        UnixMessage::Response { id, data } => app.pending.update(id, Ok(data)).await.unwrap(),
                        UnixMessage::Request { id, data } => {
                            let writer = writer.clone();
                            let app = app.clone();
                            Python::with_gil(|py| -> PyResult<()> {
                                pyo3_asyncio::tokio::future_into_py_with_locals(
                                    py,
                                    pyo3_asyncio::tokio::get_current_locals(py)?,
                                    async move { 
                                        if let Err(e) = match app.execute(data).await{
                                            Ok(data) => write_message(&mut *writer.lock().await, UnixMessage::Response { id, data }).await,
                                            Err(error) => write_message(&mut *writer.lock().await, UnixMessage::Error { id, error: error.into() }).await,
                                        }{
                                            println!("error write response request {:?}",e);
                                        }
                                        Ok(())
                                     }
                                )?;
                                Ok(())
                            }).unwrap();
                        },
                        UnixMessage::Quit => return Ok(()),
                        UnixMessage::Error { id, error } => {
                            if id == 0{
                                println!("config error : {:?}", error);
                                close_notify.notify_one();
                                return Ok(())
                            }
                            app.pending.update(id, Err(error)).await.unwrap()
                        },
                    };
                }
            }
        )?;
        Ok(())
    }).unwrap_or_default();

    {
        let close_notify = close_notify.clone();
        tokio::spawn(async move {
            signal::ctrl_c().await.unwrap();
            close_notify.notify_one();
        });
    }

    close_notify.notified().await;
    
    app.close();

    task_exchange.abort();

    Ok(())
}