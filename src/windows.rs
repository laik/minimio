use crate::ElErr;
use std::os::windows::io::{AsRawSocket, RawSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::net;
use std::io::{self, Read};

use super::MAXEVENTS;

    pub type Event = ffi::WSABUF;

    pub struct TcpStream {
        inner: net::TcpStream,
        buffer: Vec<u8>,
        status: TcpReadiness, 
    }

    enum TcpReadiness {
        Ready,
        NotReady
    }

    impl TcpStream {
        pub fn connect(adr: net::SocketAddr) -> io::Result<Self> {
            
            // This is a shortcut since this will block when establishing the connection.
            // There are several ways of avoiding this.
            // a) Obtrain the socket using system calls, set it to non_blocking 
            // b) use the crate [net2](https://docs.rs/net2/0.2.33/net2/index.html) which
            // defines a trait with default implementation for TcpStream which allow us to set
            // it to non-blocking before we connect
            let mut stream = net::TcpStream::connect(adr)?;
            stream.set_nonblocking(true)?;

            Ok(TcpStream {
                inner: stream,
                buffer: vec![1024],
                status: TcpReadiness::NotReady,
            })
        }
    }

    impl Read for TcpStream {
        fn read_to_end(&mut self, buff: &mut Vec<u8>) -> io::Result<usize> {
            match self.status {
                Ready => {
                    self.inner.read_to_end(&mut buff)
                },
                NotReady => {
                    return Err(io::Error::from(io::ErrorKind::WouldBlock));
                }
            };

            Ok(buff.len())
        }
    }

    enum ErrorTypes {
        NoError = 0,
        ChannelError = 1,
    }

    impl From<usize> for ErrorTypes {
        fn from(n: usize) -> Self {
            use ErrorTypes::*;
            match n {
                0 => NoError,
                1 => ChannelError,
                _ => panic!("Invalid error code"),
            }
        }
    }

    pub struct Selector {
        queue_handle: i32,
    }

    impl Selector {
        pub fn new() -> Result<Self, ElErr> {
            // set up the queue
            let queue_handle = ffi::create_queue()?;

            let (loop_event_tx, loop_event_rx) = channel::<ffi::IOCPevent>();

            thread::spawn(move || {
                let events = vec![ffi::IOCPevent::default(); MAXEVENTS];
                loop {
                    // TODO: wait for events

                    // handle recieved events
                    let n = 0;
                    let iocp_event = events[n].clone();
                    loop_event_tx.send(iocp_event).expect("Channel error");
                }
            });

            Ok(Selector {
                queue_handle,
            })
        }

        pub fn register_soc_read_event<T>(&mut self, soc: RawSocket) {}

        pub fn poll<T>(&mut self) -> Option<Vec<Event<T>>> {
            // calling GetQueueCompletionStatus will either return a handle to a "port" ready to read or
            // block if the queue is empty.
            None
        }

        fn check_errors(&self) -> Option<Vec<String>> {
            if self.has_error.load(Ordering::Relaxed) > 0 {
                let lock = self.errors.lock().expect("Mutex poisioned!");
                let errors = (&lock).iter().map(|s| s.clone()).collect();
                return Some(errors);
            }

            None
        }
    }

    pub enum EventResult {}

    mod ffi {
        use crate::ElErr;
        use std::os::raw::c_void;
        use std::os::windows::io::RawSocket;
        use std::ptr;

        #[derive(Debug, Clone)]
        pub struct IOCPevent {}

        impl Default for IOCPevent {
            fn default() -> Self {
                IOCPevent {}
            }
        }

        #[repr(C)]
        pub struct WSABUF {
            len: u32,
            buf: *mut u8,
        }

        impl WSABUF {
            pub fn new(len: u32, buf: *mut u8) -> Self {
                WSABUF {
                    len,
                    buf,
                }
            }
        }

        // Reference: https://docs.microsoft.com/en-us/windows/win32/api/winsock2/ns-winsock2-wsaoverlapped
        #[repr(C)]
        struct WSAOVERLAPPED {
            /// Reserved for internal use
            internal: ULONG_PTR,
            /// Reserved
            internal_high: ULONG_PTR,
            /// Reserved for service providers
            offset: DWORD,
            /// Reserved for service providers
            offset_high: DWORD,
            /// If an overlapped I/O operation is issued without an I/O completion routine
            /// (the operation's lpCompletionRoutine parameter is set to null), then this parameter
            /// should either contain a valid handle to a WSAEVENT object or be null. If the
            /// lpCompletionRoutine parameter of the call is non-null then applications are free
            /// to use this parameter as necessary.
            h_event: HANDLE,
        }

        // https://docs.microsoft.com/en-us/windows/win32/api/minwinbase/ns-minwinbase-overlapped
        struct OVERLAPPED {
            internal: ULONG_PTR,
            internal_high: ULONG_PTR,
            dummy: [DWORD; 2],
            h_event: HANDLE,
        }

        // You can find most of these here: https://docs.microsoft.com/en-us/windows/win32/winprog/windows-data-types
        /// The HANDLE type is actually a `*mut c_void` but windows preserves backwards compatibility by allowing
        /// a INVALID_HANDLE_VALUE which is `-1`. We can't express that in Rust so it's much easier for us to treat
        /// this as an isize instead;
        type HANDLE = isize;
        type DWORD = u32;
        type ULONG_PTR = *mut usize;
        type PULONG_PTR = *mut ULONG_PTR;
        type LPDWORD = *mut DWORD;
        type LPWSABUF = *mut WSABUF;
        type LPWSAOVERLAPPED = *mut WSAOVERLAPPED;
        type LPOVERLAPPED = *mut OVERLAPPED;

        // https://referencesource.microsoft.com/#System.Runtime.Remoting/channels/ipc/win32namedpipes.cs,edc09ced20442fea,references
        // read this! https://devblogs.microsoft.com/oldnewthing/20040302-00/?p=40443
        /// Defined in `win32.h` which you can find on your windows system
        static INVALID_HANDLE_VALUE: HANDLE = -1;

        // https://docs.microsoft.com/en-us/windows/win32/winsock/windows-sockets-error-codes-2
        static WSA_IO_PENDING: i32 = 997;

        // Funnily enough this is the same as -1 when interpreted as an i32
        // see for yourself: https://play.rust-lang.org/?version=stable&mode=debug&edition=2018&gist=cdb33e88acd34ef46bc052d427854210
        static INFINITE: u32 = 4294967295;

        #[link(name = "Kernel32")]
        extern "stdcall" {
            
            // https://docs.microsoft.com/en-us/windows/win32/fileio/createiocompletionport
            fn CreateIoCompletionPort(
                filehandle: HANDLE,
                existing_completionport: HANDLE,
                completion_key: ULONG_PTR,
                number_of_concurrent_threads: DWORD,
            ) -> HANDLE;
            // https://docs.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-wsarecv
            fn WSARecv(
                s: RawSocket,
                lpBuffers: LPWSABUF,
                dwBufferCount: DWORD,
                lpNumberOfBytesRecvd: LPDWORD,
                lpFlags: DWORD,
                lpOverlapped: LPWSAOVERLAPPED,
            ) -> i32;
            // https://docs.microsoft.com/en-us/windows/win32/fileio/postqueuedcompletionstatus
            fn PostQueuedCompletionStatus(
                CompletionPort: HANDLE,
                dwNumberOfBytesTransferred: DWORD,
                dwCompletionKey: ULONG_PTR,
                lpOverlapped: LPWSAOVERLAPPED,
            ) -> i32;
            // https://docs.microsoft.com/nb-no/windows/win32/api/ioapiset/nf-ioapiset-getqueuedcompletionstatus
            fn GetQueuedCompletionStatus(
                CompletionPort: HANDLE,
                lpNumberOfBytesTransferred: LPDWORD,
                lpCompletionKey: PULONG_PTR,
                lpOverlapped: LPOVERLAPPED,
                dwMilliseconds: DWORD,
            ) -> i32;
            // https://docs.microsoft.com/nb-no/windows/win32/api/handleapi/nf-handleapi-closehandle
            fn CloseHandle(hObject: HANDLE) -> i32;

            // https://docs.microsoft.com/nb-no/windows/win32/api/winsock/nf-winsock-wsagetlasterror
            fn WSAGetLastError() -> i32;
        }

        // ===== SAFE WRAPPERS =====

        pub fn create_queue() -> Result<i32, ElErr> {
            unsafe {
                // number_of_concurrent_threads = 0 means use the number of physical threads but the argument is
                // ignored when existing_completionport is set to null.
                let res = CreateIoCompletionPort(INVALID_HANDLE_VALUE, 0, ptr::null_mut(), 0);
                if (res as *mut usize).is_null() {
                    return Err(std::io::Error::last_os_error().into());
                }
                Ok(*(res as *const i32))
            }
        }

        pub fn create_soc_read_event(s: RawSocket, wsabuffers: &mut [WSABUF], bytes_recieved: &mut u32, ol: &mut WSAOVERLAPPED) -> Result<(), ElErr> {
            // This actually takes an array of buffers but we will only need one so we can just box it
            // and point to it (there is no difference in memory between a `vec![T; 1]` and a `Box::new(T)`)
            let buff_ptr: *mut WSABUF = wsabuffers.as_mut_ptr();
            //let num_bytes_recived_ptr: *mut u32 = bytes_recieved;

       
                let res = unsafe { WSARecv(s, buff_ptr, 1, bytes_recieved, 0, ol) };

                    if res != 0 {
                    let err = unsafe { WSAGetLastError() };

                    if err == WSA_IO_PENDING {
                        // Everything is OK, and we can wait this with GetQueuedCompletionStatus
                        Ok(())
                    } else {
                        return Err(std::io::Error::last_os_error().into());
                    }

                } else {
                    // The socket is already ready so we don't need to queue it
                    // TODO: Avoid queueing this
                    Ok(())
                }
            }

        pub fn register_event(completion_port: isize, bytes_to_transfer: u32, completion_key: &mut usize, overlapped_ptr: &mut WSAOVERLAPPED) -> Result<(), ElErr> {
            let res = unsafe { PostQueuedCompletionStatus(completion_port, bytes_to_transfer, completion_key, overlapped_ptr)};
            if res != 0 {
                Err(std::io::Error::last_os_error().into())
            } else {
                Ok(())
            }
        }


        pub fn get_queued_completion_status(completion_port: isize, bytes_transferred_ptr: &mut u32, completion_key_ptr: &mut &mut usize, overlapped_ptr: *mut OVERLAPPED) -> Result<(), ElErr> {
            // can't coerce directly to *mut *mut usize and cant cast `&mut` as `*mut`
            let completion_key_ptr: *mut &mut usize = completion_key_ptr;
            // but we can cast a `*mut ...`
            let completion_key_ptr: *mut *mut usize = completion_key_ptr as *mut *mut usize;
            let res = unsafe { GetQueuedCompletionStatus(completion_port, bytes_transferred_ptr, completion_key_ptr, overlapped_ptr, INFINITE)};

            if res != 0 {
                Err(std::io::Error::last_os_error().into())
            } else {
                Ok(())
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            #[test]
            fn create_queue_works() {
                let queue = create_queue().unwrap();
                assert!(queue > 0);
            }
        }
    }