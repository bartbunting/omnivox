use super::*;

/// Bound the request write as well as the response wait, and join the watchdog
/// before releasing lifecycle ownership. No timer survives a completed exchange.
pub(super) fn with_connection_deadline<T>(
    connection: &Arc<dyn HelperConnection>,
    deadline: Instant,
    operation: &'static str,
    query: impl FnOnce(Instant) -> Result<T, HelperEngineError>,
) -> Result<T, HelperEngineError> {
    if Instant::now() >= deadline {
        return Err(HelperEngineError::Timeout(operation));
    }
    const ACTIVE: u8 = 0;
    const COMPLETED: u8 = 1;
    const EXPIRED: u8 = 2;
    let state = Arc::new(AtomicU8::new(ACTIVE));
    let (finished, waiting) = mpsc::sync_channel(1);
    let watched = Arc::clone(connection);
    let watched_state = Arc::clone(&state);
    let watchdog = thread::Builder::new()
        .name("omnivox-helper-deadline".into())
        .spawn(move || {
            if matches!(
                waiting.recv_timeout(deadline.saturating_duration_since(Instant::now())),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) && watched_state
                .compare_exchange(ACTIVE, EXPIRED, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // Termination kills before taking the writer lock; a full pipe wakes.
                if let Err(error) = watched.terminate() {
                    warn!(operation, %error, "Deadline could not confirm helper cleanup");
                }
            }
        })
        .map_err(|e| HelperEngineError::Transport(format!("could not start deadline: {e}")))?;
    let result = query(deadline);
    if Instant::now() >= deadline {
        if state
            .compare_exchange(ACTIVE, EXPIRED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let _ = connection.terminate();
        }
    } else {
        let _ = state.compare_exchange(ACTIVE, COMPLETED, Ordering::AcqRel, Ordering::Acquire);
    }
    let _ = finished.send(());
    watchdog
        .join()
        .map_err(|_| HelperEngineError::Transport("deadline worker panicked".into()))?;
    if state.load(Ordering::Acquire) == EXPIRED {
        return Err(HelperEngineError::Timeout(operation));
    }
    result
}

pub(super) fn initialization_exchange(
    connection: &Arc<dyn HelperConnection>,
    request: &HelperRequest,
    timeout: Duration,
    admission_deadline: Option<Instant>,
) -> Result<HelperResponse, HelperEngineError> {
    let operation_deadline = Instant::now() + timeout;
    let (deadline, operation) = match admission_deadline {
        Some(deadline) if deadline <= operation_deadline => (deadline, "external startup budget"),
        _ => (operation_deadline, "helper initialization"),
    };
    with_connection_deadline(connection, deadline, operation, |deadline| {
        connection.send(request)?;
        receive_owned_response(
            connection,
            request.request_id,
            deadline.saturating_duration_since(Instant::now()),
        )
    })
}
