use std::sync::mpsc;

use orch_protocol::RequestError;
use orch_store::Store;
use tokio::sync::oneshot;

type Job = Box<dyn FnOnce(&mut Store) + Send>;

#[derive(Clone)]
pub(crate) struct StoreHandle {
    jobs: mpsc::Sender<Job>,
}

impl StoreHandle {
    pub(crate) fn spawn(mut store: Store) -> Self {
        let (jobs, queue) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("orch-store".into())
            .spawn(move || {
                for job in queue {
                    job(&mut store);
                }
            })
            .expect("the store thread starts");
        Self { jobs }
    }

    pub(crate) fn write(&self, job: impl FnOnce(&mut Store) + Send + 'static) {
        let _ = self.jobs.send(Box::new(job));
    }

    pub(crate) async fn call<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Store) -> T + Send + 'static,
    ) -> Result<T, RequestError> {
        let (reply, answer) = oneshot::channel();
        self.write(move |store| {
            let _ = reply.send(job(store));
        });
        answer.await.map_err(|_| gone())
    }

    pub(crate) fn call_blocking<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Store) -> T + Send + 'static,
    ) -> Result<T, RequestError> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.write(move |store| {
            let _ = reply.send(job(store));
        });
        answer.recv().map_err(|_| gone())
    }
}

fn gone() -> RequestError {
    RequestError::Refused {
        message: "the state database is unavailable".into(),
    }
}
