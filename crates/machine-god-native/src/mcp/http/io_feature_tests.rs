use super::*;
use crate::mcp::control::tests::human_selected;
use tokio::io::AsyncReadExt;
struct Clock;
impl super::super::McpHttpClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}

#[test]
fn feature_writer_checks_retirement_at_each_suffix_and_flush() {
    super::super::tests::executor().block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (outgoing, incoming) = futures_util::future::join(
            tokio::net::TcpStream::connect(listener.local_addr().unwrap()),
            listener.accept(),
        )
        .await;
        let mut stream = Stream::Plain(outgoing.unwrap());
        let (mut incoming, _) = incoming.unwrap();
        let token = CancellationToken::new();
        let retired = Arc::new(AtomicBool::new(false));
        let observation = McpHttpObservation(Arc::new(Observation::default()));
        let mut writer = Writer {
            stream: &mut stream,
            observation: &observation,
            cancellation: CancellationToken::new(),
            deadline: Instant::now() + std::time::Duration::from_secs(5),
            clock: Arc::new(Clock),
            feature: Some(human_selected(token.clone(), retired.clone())),
        };
        let count = poll_fn(|cx| writer.poll_write(cx, b"prefix"))
            .await
            .unwrap();
        assert_eq!(count, 6);
        let mut bytes = [0; 6];
        incoming.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"prefix");
        retired.store(true, Ordering::Release);
        assert!(!token.is_cancelled());
        assert!(
            poll_fn(|cx| writer.poll_write(cx, b"suffix"))
                .await
                .is_err()
        );
        assert!(poll_fn(|cx| writer.poll_flush(cx)).await.is_err());
        assert_eq!(observation.0.acknowledged.load(Ordering::Acquire), 6);
    });
}
