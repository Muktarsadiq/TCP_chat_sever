use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream}; // Added SocketAddr for clarity in HashMap
use std::result;
use std::io::{self, Write, Read}; // Added io for ErrorKind
use std::thread;
use std::sync::{mpsc, Arc}; // Arc is the Atomic Reference Counted pointer, crucial for sharing data across threads.
use std::sync::mpsc::{Receiver, Sender}; // For creating the communication channel between threads.
use std::time::{Duration, Instant};


// Type alias for Result to simplify function signatures.
// Using () as the error type means we care *that* an error happened,
// but not necessarily the specific error type in the function signature.
// Errors are typically logged before being converted to ().
type Result<T> = result::Result<T, ()>;

// Global flag to control sensitive data redaction.
static SAFE_MODE: bool = false;
// Ban Limit for banned clients. 
static BAN_LIMIT: Duration = Duration::new(60 * 10, 0); 
// Set rate limit  
static MESSAGE_RATE_LIMIT: Duration = Duration::from_millis(500); 
// Set Max strikes; 
static MAX_STRIKES: u8 = 5; 

// Wrapper struct to potentially redact sensitive information when displayed.
struct Sensitive<T>(T);

// Implementation of the Display trait for the Sensitive wrapper.
impl<T: std::fmt::Display> std::fmt::Display for Sensitive<T>{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>)-> std::fmt::Result{
        let Self(inner) = self; // Destructure the inner value.
        if SAFE_MODE {
            writeln!(f, "[REDACTED]") // Print redacted if safe mode is on.
        }else{
            writeln!(f, "{inner}") // Otherwise, print the inner value using its Display impl.
        }
    }
}

// Function executed by each client thread.
// Takes ownership of an Arc<TcpStream> representing the client's connection
// and a Sender channel half to communicate back to the server thread.
fn client(stream: Arc<TcpStream>, messages: Sender<Message>) -> Result<()>{
    // Immediately notify the server thread that a new client has connected.
    // We clone the Arc<TcpStream> again here. This increases the reference count,
    // allowing the server thread to also hold a reference to the same stream.
    messages.send(Message::ClientConnected(stream.clone())).map_err(|err|{
        // Log error if the message channel is broken (server thread likely panicked).
        eprintln!("ERROR: could not send message to reciever: {err}");
    })?;

    // Buffer to store data read from the client stream.
    let mut buffer = vec![0; 64]; // 64-byte buffer.

    // Loop indefinitely, reading data from the client.
    loop {
        // Attempt to read data from the stream into the buffer.
        // stream is Arc<TcpStream>. We need a reference to the inner TcpStream (&TcpStream)
        // to call read(). `as_ref()` provides this immutable reference.
        // Note: `stream.read()` would also work due to Deref coercion.
        match stream.as_ref().read(&mut buffer) {
            // Ok(0) signifies that the client has gracefully closed the connection
            // from their end (like an 'end' event in other systems).
            Ok(0) => {
                println!("INFO: Client disconnected gracefully.");
                // Notify the server thread about the disconnection.
                // We clone the Arc again to send it in the message.
                if let Ok(socket_addr) = stream.peer_addr(){
                    let _ = messages.send(Message::ClientDisconnected(socket_addr));
                    break; // Exit the loop, allowing the client thread to terminate.
                }
            }

            // Ok(n) means 'n' bytes were successfully read into the buffer.
            Ok(n) => {
                // Get client's socket address. 
                if let Ok(socket_addr) = stream.peer_addr(){
                    // Send the received data (as a new Vec<u8>) along with a clone
                    // of the stream's Arc to the server thread.                    
                    messages.send(Message::NewMessage(socket_addr, buffer[0..n].to_vec())).map_err(|err|{
                        eprintln!("ERROR: could not send message to receiver: {err}");
                    })?;                    
                }
                
            }
            // Err(e) indicates a read error. We need to check the error kind.
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
                // These errors are expected if using non-blocking I/O or read timeouts.
                // `WouldBlock` (EAGAIN/EWOULDBLOCK) means the OS call would block,
                // and `TimedOut` means the read timeout set in main() was reached.
                // In either case, the connection is likely still valid, so we just continue
                // the loop and try reading again later.
                continue;
            }
            // Any other error is treated as a fatal disconnection.
            Err(err) => {
                eprintln!("ERROR: could not read message from client: {err}");
                // Get socket address for disconnecting client. 
                if let Ok(socket_addr) = stream.peer_addr(){
                    // Notify the server thread about the disconnection due to error.
                    let _ = messages.send(Message::ClientDisconnected(socket_addr));
                    // Return an error to terminate this client thread.
                    // The `?` operator cannot be used here as we need to send the disconnect message first.
                    return Err(());                    
                }

            }
        }
    }
    // If the loop exits gracefully (only via Ok(0)), return Ok.
    Ok(())
}

// Enum defining the types of messages client threads can send to the server thread.
#[derive(Debug)]
enum Message{
    // Sent when a client thread starts. Contains a clone of the client's Arc<TcpStream>.
    ClientConnected(Arc<TcpStream>),
    // Sent when a client disconnects (gracefully or due to error). Contains a clone of the client's Arc<TcpStream>.
    ClientDisconnected(SocketAddr),
    // Sent when a client sends data. Contains a clone of the sender's Arc<TcpStream> and the data bytes.
    NewMessage(SocketAddr,Vec<u8>)
}

// Simple struct to hold client information managed by the server thread.
// Currently holds the Arc<TcpStream> for writing back to the client, Instant for the last message sent and a u8 for the strike count.
struct Client{
    conn: Arc<TcpStream>,
    last_message: Instant,
    strike_count: u8, 
}

// Function executed by the single server thread.
// Takes ownership of the Receiver channel half.
fn server(message: Receiver<Message>) -> Result<()>{
    let mut clients: HashMap<SocketAddr, Client> = HashMap::new();
    let mut banned_clients: HashMap<IpAddr, Instant> = HashMap::new();

    loop {
        let msg = message.recv().expect("The server receiver is not hung up");

        match msg {
            Message::ClientConnected(author_stream) => {
                // Use a block to scope the mutable borrow of banned_clients
                let maybe_banned_info = {
                    let author_addr = match author_stream.peer_addr() {
                        Ok(addr) => addr,
                        Err(_) => {
                            eprintln!("ERROR: Could not get peer address for connecting client. Disconnecting.");
                            let _ = author_stream.shutdown(std::net::Shutdown::Both);
                            continue; // Skip processing this client
                        }
                    };
                    println!("INFO: Client connecting: {}", Sensitive(author_addr));
                    let ip = author_addr.ip();
                    let now = Instant::now();

                    // Check if banned and if ban expired
                    if let Some(banned_at) = banned_clients.get(&ip) {
                        if now.duration_since(*banned_at) < BAN_LIMIT {
                            // Still banned
                            Some((author_addr, true)) // Banned = true
                        } else {
                            // Ban expired, remove from ban list
                            println!("INFO: Ban expired for IP {}", Sensitive(ip));
                            banned_clients.remove(&ip);
                            Some((author_addr, false)) // Banned = false
                        }
                    } else {
                        // Not currently banned
                        Some((author_addr, false)) // Banned = false
                    }
                }; // End of block, mutable borrow of banned_clients released

                if let Some((author_addr, is_banned)) = maybe_banned_info {
                    if is_banned {
                        println!("INFO: Rejecting banned client {}", Sensitive(author_addr));
                        let _ = author_stream.as_ref().write(b"You are still temporarily banned.\n");
                        let _ = author_stream.shutdown(std::net::Shutdown::Both);
                    } else {
                        // Client is not banned (or ban expired), add to active clients
                        println!("INFO: Client accepted: {}", Sensitive(author_addr));
                        clients.insert(author_addr, Client {
                            conn: author_stream,
                            last_message: Instant::now(), // Initialize last_message time
                            strike_count: 0,
                        });
                    }
                }
            },

            Message::ClientDisconnected(author_addr) => {
                println!("INFO: Client disconnected: {}", Sensitive(author_addr));
                // Remove the client from the active list.
                clients.remove(&author_addr);
                // Note: No need to remove from banned_clients here, that happens on attempted reconnect.
            },

            Message::NewMessage(author_addr, bytes) => {
                let now = Instant::now();
                let mut should_broadcast = true; // Assume we broadcast unless rate limited or banned

                // Get a mutable reference to the author to check/update state
                if let Some(author) = clients.get_mut(&author_addr) {

                    // --- Rate Limit & Strike Logic ---
                    let time_since_last = now.duration_since(author.last_message);

                    if time_since_last < MESSAGE_RATE_LIMIT {
                        // Rate limit violated
                        author.strike_count = author.strike_count.saturating_add(1); // Safely add strike
                        println!(
                            "INFO: Client {} received strike {}/{} (rate limit)",
                            Sensitive(author_addr), author.strike_count, MAX_STRIKES
                        );
                        should_broadcast = false; // Don't broadcast this message

                        if author.strike_count >= MAX_STRIKES {
                            // Ban the client
                            println!("INFO: Banning client {} for exceeding strike limit.", Sensitive(author_addr));
                            banned_clients.insert(author_addr.ip(), now);

                            // Inform client (best effort) and shutdown connection
                            let _ = author.conn.as_ref().write(b"You have been banned for sending messages too quickly.\n");
                            let _ = author.conn.shutdown(std::net::Shutdown::Both);

                            // Remove from active clients *immediately* after banning
                            clients.remove(&author_addr);
                            // Skip further processing for this message
                            continue;
                        }
                    } else {
                        // Rate limit not violated, decrement strike count on good behaviour.
                        author.strike_count = author.strike_count.saturating_sub(1);

                        // Update last_message time ONLY if the message wasn't rate-limited
                        author.last_message = now;
                    }
                    // --- End Rate Limit & Strike Logic ---

                } else {
                    // Author not found in active clients (maybe already disconnected/banned?)
                    eprintln!("WARN: Received message from unknown or disconnected client {}", Sensitive(author_addr));
                    should_broadcast = false; // Don't broadcast if we don't know the sender
                }


                // --- Broadcasting Logic ---
                if should_broadcast {
                    // Iterate immutably over clients to broadcast
                    for (recipient_addr, recipient) in clients.iter() {
                        // Don't send the message back to the original sender
                        if *recipient_addr != author_addr {
                            // Broadcast the original bytes
                            let _ = recipient.conn.as_ref().write(&bytes).map_err(|err| {
                                eprintln!("ERROR: could not write to client {}: {}", Sensitive(recipient_addr), Sensitive(err));
                                // TODO: Consider removing recipient if write fails repeatedly
                            });
                        }
                    }
                }
                // --- End Broadcasting Logic ---
            }
        }
    }
    // Ok(()) // Unreachable
}

// Main function - entry point of the application.
fn main()-> Result<()> {
    // Define the address to listen on.
    // 0.0.0.0 means listen on all available network interfaces (IPv4).
    // 4567 is the port number.
    let address = "0.0.0.0:4567";

    // Attempt to bind the TcpListener to the specified address.
    // `bind()` returns a Result<TcpListener, io::Error>.
    // `map_err()` is used to log the error before converting it to the () error type.
    let listener = TcpListener::bind(address).map_err(|err|{
        eprintln!("ERROR: could not bind {address}: {err}", address = Sensitive(address), err = Sensitive(err));
    })?;

    // Log that the server is listening.
    println!("INFO: Listening on adress: {address}", address = Sensitive(address));

    // Create the Multi-Producer, Single-Consumer (MPSC) channel.
    // `sender` will be cloned for each client thread (Multi-Producer).
    // `reciever` will be moved to the single server thread (Single-Consumer).
    let (sender, reciever) = mpsc::channel::<Message>();

    // Spawn the single server thread.
    // `thread::spawn` takes a closure. The closure takes ownership of `reciever`.
    // The server thread will run the `server` function.
    thread::spawn(|| server(reciever)); // Note: Result of server function is ignored here.

    // Main loop: Accept incoming connections.
    // `listener.incoming()` returns an iterator that blocks until a new connection arrives.
    for stream_result in listener.incoming(){
        // Match on the result of accepting a connection.
        match stream_result{
            // Successfully accepted a new connection -> `stream` is a TcpStream.
            Ok(stream) => {
               // Set a read timeout on the stream.
               // If no data is received for 5 seconds during a `read` call,
               // the `read` will return an `Err` with kind `TimedOut`.
               // This prevents client threads from blocking indefinitely.
               let _ = stream.set_read_timeout(Some(Duration::new(5,0)));

               // Clone the sender part of the channel.
               // Each client thread needs its own Sender handle to send messages
               // back to the single server thread's Receiver. Cloning is cheap.
               let message_sender = sender.clone();

               // Wrap the TcpStream in an Arc (Atomic Reference Counted pointer).
               // This allows the stream to be safely shared across threads.
               // - The client thread needs it for reading.
               // - The server thread needs it (via Messages) for writing and tracking.
               // `Arc::new()` creates the Arc and takes ownership of the stream.
               let stream_arc = Arc::new(stream);

               // Spawn a new thread for this specific client.
               // `thread::spawn` takes a closure. The closure takes ownership of
               // the `stream_arc` (the Arc<TcpStream>) and the `message_sender`.
               // The new thread will execute the `client` function.
               thread::spawn(||client(stream_arc, message_sender)); // Note: Result of client function is ignored here.
            }
            // An error occurred while accepting the connection.
            Err(e) => {
                eprintln!("ERROR: could not accept stream: {e}");
                // Continue the loop to try accepting the next connection.
                continue;
            }
        }
    }

    // This part is unreachable because listener.incoming() loops forever.
    Ok(())
}