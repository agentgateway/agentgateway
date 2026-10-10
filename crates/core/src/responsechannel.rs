use tokio::sync::{mpsc, oneshot};

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
	pub async fn send_and_wait(&self, request: T) -> anyhow::Result<R> {
		let (response_tx, response_rx) = oneshot::channel();
		self
			.tx
			.send((request, response_tx))
			.await
			.map_err(|_| anyhow::anyhow!("tx channel closed"))?;
		response_rx
			.await
			.map_err(|_| anyhow::anyhow!("rx channel closed"))
	}
}

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
