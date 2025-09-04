use tokio::sync::mpsc::error::SendError;
use tokio::sync::{mpsc, oneshot};

pub type AckSender<T> = Sender<T, ()>;
#[derive(Debug)]
pub struct Sender<T, R> {
	tx: mpsc::Sender<(T, oneshot::Sender<R>)>,
}

impl<T, R> Clone for Sender<T, R> {
	fn clone(&self) -> Self {
		Self {
			tx: self.tx.clone(),
		}
	}
}

pub fn new<T, R>(buffer: usize) -> (Sender<T, R>, Receiver<T, R>) {
	let (tx, rx) = mpsc::channel(buffer);
	let channel = Sender { tx };
	let handler = Receiver { rx };
	(channel, handler)
}

impl<T, R> Sender<T, R>
where
	T: Send + 'static,
	R: Send + 'static,
{
	pub fn blocking_send_and_wait(&self, request: T) -> Result<R, oneshot::error::RecvError> {
		let tx = self.tx.clone();
		// TODO: this is unacceptable
		std::thread::spawn(move || {
			let (response_tx, response_rx) = oneshot::channel();
			tx.blocking_send((request, response_tx)).unwrap();
			response_rx.blocking_recv()
		})
		.join()
		.unwrap()
	}
	pub async fn send_and_wait(&self, request: T) -> Result<R, oneshot::error::RecvError> {
		let (response_tx, response_rx) = oneshot::channel();
		self.tx.send((request, response_tx)).await.unwrap();
		response_rx.await
	}
	pub async fn send_ignore(&self, request: T) -> Result<(), SendError<(T, oneshot::Sender<R>)>> {
		let (response_tx, _) = oneshot::channel();
		self.tx.send((request, response_tx)).await
	}
}

pub type AckReceiver<T> = Receiver<T, ()>;
pub struct Receiver<T, R> {
	rx: mpsc::Receiver<(T, oneshot::Sender<R>)>,
}

impl<T, R> Receiver<T, R>
where
	T: Send + 'static,
	R: Send + 'static,
{
	pub async fn recv(&mut self) -> Option<(T, oneshot::Sender<R>)> {
		self.rx.recv().await
	}
}
