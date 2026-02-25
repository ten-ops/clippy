/// A common interface for clipboard backends that abstracts platform-specific clipboard operations.
///
/// This trait defines the core operations required to connect to a system clipboard,
/// listen for changes, and retrieve clipboard data. Implementations are responsible
/// for handling platform-specific details such as event loops, data formats, and error
/// conditions.
///
/// # Associated Type
/// - `Error`: The error type returned by backend operations. It should be `Send + Sync` if the
///   backend needs to be used across threads, though this is not enforced by the trait.
pub trait ClipboardBackend: Sized {
    /// The error type for clipboard operations.
    type Error;

    /// Establishes a connection to the system clipboard.
    ///
    /// This method initializes the clipboard backend, performing any necessary setup such as
    /// opening a display connection, initializing platform APIs, or acquiring resources.
    /// It should be called before using `run` or other operations.
    ///
    /// # Returns
    /// - `Ok(Self)` on successful connection, containing a new instance of the backend.
    /// - `Err(Self::Error)` if the connection fails due to platform-specific issues (e.g.,
    ///   missing display server, permission denied, or unsupported environment).
    ///
    /// # Errors
    /// This method may fail if the underlying system clipboard is unavailable or inaccessible.
    /// The specific error conditions are defined by the implementing type.
    fn connect() -> Result<Self, Self::Error>;

    /// Enters the clipboard event loop, invoking a handler for each clipboard update.
    ///
    /// This method blocks the current thread and continuously monitors the clipboard for
    /// changes. Whenever new data becomes available, the provided `handler` is called with
    /// the raw clipboard content as a byte vector. The method returns only when an error
    /// occurs or the backend is shut down (platform-dependent).
    ///
    /// # Parameters
    /// - `handler`: A closure or function that will be called on every clipboard change.
    ///   It receives a `Vec<u8>` containing the raw clipboard data. The handler should
    ///   process the data quickly to avoid blocking the event loop; heavy processing
    ///   should be offloaded to another thread.
    ///
    /// # Returns
    /// - `Ok(())` if the event loop exits cleanly (e.g., on explicit shutdown).
    /// - `Err(Self::Error)` if an error occurs while monitoring the clipboard.
    ///
    /// # Errors
    /// Implementations may return errors for issues such as loss of connection to the
    /// clipboard service, invalid data, or resource exhaustion.
    ///
    /// # Notes
    /// - The `handler` may be called from a platform-specific thread; ensure it is safe
    ///   to call from that context (e.g., `Send` if threading is involved). This trait
    ///   does not impose threading constraints, but implementations should document their
    ///   behavior.
    /// - The byte vector represents the clipboard content in a platform-defined format.
    ///   Typically, it may be UTF-8 text, but could also contain binary data (e.g., images).
    ///   The caller must interpret the data accordingly.
    /// - This method is expected to run indefinitely; consider using a separate thread
    ///   if non-blocking behavior is required.
    fn run<F>(&mut self, handler: F) -> Result<(), Self::Error>
    where
        F: FnMut(Vec<u8>);

    /// Determines whether a given error is fatal for the clipboard backend.
    ///
    /// This associated function allows the caller to decide if an error returned by
    /// `connect` or `run` indicates a permanent failure that requires reinitialization
    /// or termination, versus a transient error that might be resolved by retrying.
    ///
    /// # Parameters
    /// - `err`: A reference to an error returned by a backend operation.
    ///
    /// # Returns
    /// - `true` if the error is fatal (e.g., the clipboard service is unavailable,
    ///   permissions are permanently denied, or the platform is unsupported).
    /// - `false` if the error might be temporary (e.g., resource contention,
    ///   temporary connection loss) and the operation could be retried later.
    ///
    fn is_fatal_error(err: &Self::Error) -> bool;
}
