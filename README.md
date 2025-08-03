# TCP_chat_sever

# TCP Chat Server

A simple, multi-threaded TCP chat server written in Rust that enables real-time messaging between connected clients with built-in rate limiting and ban management.

## Features

- **Multi-threaded Architecture**: Each client connection runs in its own thread for concurrent handling
- **Real-time Message Broadcasting**: Messages from one client are instantly broadcast to all other connected clients
- **Rate Limiting**: Prevents message spam with configurable rate limits (500ms between messages by default)
- **Strike System**: Clients receive strikes for rate limit violations, with automatic banning after 5 strikes
- **Temporary IP Banning**: Clients that exceed strike limits are banned for 10 minutes
- **Connection Management**: Graceful handling of client connections and disconnections
- **Sensitive Data Protection**: Optional redaction mode for logging sensitive information
- **Timeout Handling**: 5-second read timeouts prevent threads from blocking indefinitely

## Technical Specifications

### Configuration Constants

- **Port**: 4567 (listening on all interfaces: `0.0.0.0:4567`)
- **Rate Limit**: 500ms minimum between messages
- **Max Strikes**: 5 strikes before ban
- **Ban Duration**: 10 minutes
- **Read Timeout**: 5 seconds
- **Buffer Size**: 64 bytes per read operation

### Architecture

- **Server Thread**: Single thread managing all client state and message broadcasting
- **Client Threads**: One thread per connected client for handling I/O
- **Communication**: MPSC (Multi-Producer, Single-Consumer) channels for thread communication
- **Memory Management**: Arc (Atomic Reference Counting) for safe cross-thread data sharing

## Building and Running

### Prerequisites

- Rust 1.70+ (uses stable features only)
- No external dependencies beyond std library

### Build

```bash
cargo build --release
```

### Run

```bash
cargo run
```

The server will start listening on `0.0.0.0:4567` and display:

```
INFO: Listening on address: 0.0.0.0:4567
```

## Usage

### Connecting Clients

Connect to the server using any TCP client (telnet, netcat, custom client):

```bash
# Using telnet
telnet localhost 4567

# Using netcat
nc localhost 4567
```

### Messaging

- Simply type messages and press Enter
- Messages are automatically broadcast to all other connected clients
- The sender does not receive their own message back

## Limitations

### Protocol Limitations

- **No Message Format**: Raw bytes are transmitted without any protocol structure
- **No User Authentication**: No login system or user identification
- **No Message History**: Messages are not persisted; only live broadcasting
- **No Private Messages**: All messages are broadcast to everyone
- **Limited Message Size**: 64-byte buffer per read operation (messages can be fragmented)

### Scalability Limitations

- **Memory Usage**: All client connections held in memory simultaneously
- **No Persistence**: Client state and ban list lost on server restart
- **Single Server**: No clustering or load balancing support
- **Blocking Operations**: Some operations may impact performance under high load

### Security Limitations

- **No Encryption**: All communication is in plaintext
- **Basic Rate Limiting**: Simple time-based rate limiting (no sophisticated flood protection)
- **IP-based Banning**: Can be bypassed with IP changes
- **No Input Validation**: Raw bytes are accepted and broadcast without filtering
- **No Access Controls**: Anyone who can connect to the port can participate

### Error Handling Limitations

- **Limited Recovery**: Most errors result in client disconnection
- **No Graceful Shutdown**: Server must be forcibly terminated
- **Minimal Logging**: Basic logging without structured log levels or external logging

## Configuration

### Compile-time Constants

To modify server behavior, edit these constants in `main.rs`:

```rust
static SAFE_MODE: bool = false;                           // Enable sensitive data redaction
static BAN_LIMIT: Duration = Duration::new(60 * 10, 0);   // Ban duration (10 minutes)
static MESSAGE_RATE_LIMIT: Duration = Duration::from_millis(500); // Rate limit (500ms)
static MAX_STRIKES: u8 = 5;                               // Strikes before ban
```

### Runtime Configuration

- Change the listening address by modifying the `address` variable in `main()`
- Adjust read timeout by modifying the `set_read_timeout()` call

## Architecture Details

### Thread Model

```
Main Thread
├── Server Thread (message processing & broadcasting)
└── Client Threads (one per connection)
    ├── Client Thread 1
    ├── Client Thread 2
    └── Client Thread N
```

### Message Flow

1. Client sends data → Client Thread reads it
2. Client Thread → Server Thread (via MPSC channel)
3. Server Thread processes and broadcasts to all other clients
4. Server Thread writes to recipient Client Threads

### State Management

- **Active Clients**: HashMap<SocketAddr, Client> - tracks connected clients
- **Banned Clients**: HashMap<IpAddr, Instant> - tracks banned IPs with ban timestamps
- **Client State**: Connection handle, last message time, strike count

## Contributing

This is a basic implementation suitable for learning purposes. Potential improvements:

- Add message encryption (TLS/SSL)
- Implement proper protocol with message framing
- Add user authentication and authorization
- Implement message persistence and history
- Add configuration file support
- Improve error handling and recovery
- Add metrics and structured logging
- Implement graceful shutdown handling

## License

MIT
This project is provided as-is for educational purposes.
