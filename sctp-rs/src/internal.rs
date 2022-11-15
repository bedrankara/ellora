//! Actual implementation of the API Calls
//!
//! Nothing in this module should be public API as this module contains `unsafe` code that uses
//! `libc` and internal `libc` structs and function calls.

use std::convert::TryInto;
use std::net::SocketAddr;
use std::os::unix::io::{AsRawFd, RawFd};

use os_socketaddr::OsSocketAddr;

use crate::types::internal::{SctpEventSubscribe, SctpGetAddrs};
use crate::{
    AssociationChange, BindxFlags, SctpAssociationId, SctpConnectedSocket, SctpEvent,
    SctpNotification, SctpNotificationOrData,
};

#[allow(unused)]
use super::consts::*;

static SOL_SCTP: libc::c_int = 132;

/// Implementation of `sctp_bindx` using `libc::setsockopt`
pub(crate) fn sctp_bindx_internal(
    fd: RawFd,
    addrs: &[SocketAddr],
    flags: BindxFlags,
) -> std::io::Result<()> {
    let mut addrs_u8: Vec<u8> = vec![];

    for addr in addrs {
        let ossockaddr: OsSocketAddr = (*addr).into();
        let slice = ossockaddr.as_ref();
        addrs_u8.extend(slice);
    }

    let addrs_len = addrs_u8.len();

    let flags = match flags {
        BindxFlags::Add => SCTP_SOCKOPT_BINDX_ADD,
        BindxFlags::Remove => SCTP_SOCKOPT_BINDX_REM,
    };

    eprintln!(
        "addrs_len: {}, addrs_u8: {:?}, flags: {}",
        addrs_len, addrs_u8, flags
    );

    // Safety: The passed vector is valid during the function call and hence the passed reference
    // to raw data is valid.
    unsafe {
        let result = libc::setsockopt(
            fd,
            SOL_SCTP,
            flags,
            addrs_u8.as_ptr() as *const _ as *const libc::c_void,
            addrs_len as libc::socklen_t,
        );

        if result < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

/// Implementation of `sctp_peeloff` using `libc::getsockopt`
pub(crate) fn sctp_peeloff_internal(
    fd: RawFd,
    assoc_id: SctpAssociationId,
) -> std::io::Result<RawFd> {
    use crate::types::internal::SctpPeeloffArg;

    let mut peeloff_arg = SctpPeeloffArg::from_assoc_id(assoc_id);
    let mut peeloff_size: libc::socklen_t =
        std::mem::size_of::<SctpPeeloffArg>() as libc::socklen_t;

    // Safety Pointer to `peeloff_arg` and `peeloff_size` is valid as the variable is still in the
    // scope
    unsafe {
        let peeloff_arg_ptr = std::ptr::addr_of_mut!(peeloff_arg);
        let peeloff_size_ptr = std::ptr::addr_of_mut!(peeloff_size);
        let result = libc::getsockopt(
            fd,
            SOL_SCTP,
            SCTP_SOCKOPT_PEELOFF,
            peeloff_arg_ptr as *mut _ as *mut libc::c_void,
            peeloff_size_ptr as *mut _ as *mut libc::socklen_t,
        );
        if result < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(peeloff_arg.sd.as_raw_fd())
        }
    }
}

/// Implementation of `socket` using `libc::socket`.
///
/// Based on the type of the requested socket, we pass different `type` parameter to actual
/// `libc::socket` call. See section 3.1.1 and section 4.1.1 of RFC 6458.
pub(crate) fn sctp_socket_internal(
    domain: libc::c_int,
    assoc: crate::SocketToAssociation,
) -> RawFd {
    unsafe {
        match assoc {
            crate::SocketToAssociation::OneToOne => {
                libc::socket(domain, libc::SOCK_STREAM, libc::IPPROTO_SCTP)
            }
            crate::SocketToAssociation::OneToMany => {
                libc::socket(domain, libc::SOCK_SEQPACKET, libc::IPPROTO_SCTP)
            }
        }
    }
}

/// Implementation of `listen` using `libc::listen`
pub(crate) fn sctp_listen_internal(fd: RawFd, backlog: i32) -> std::io::Result<()> {
    unsafe {
        let result = libc::listen(fd, backlog);

        if result < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

/// Implmentation of `sctp_getpaddrs` using `libc::getsockopt`
pub(crate) fn sctp_getpaddrs_internal(
    fd: RawFd,
    assoc_id: SctpAssociationId,
) -> std::io::Result<Vec<SocketAddr>> {
    sctp_getaddrs_internal(fd, SCTP_GET_PEER_ADDRS, assoc_id)
}

/// Implmentation of `sctp_getladdrs` using `libc::getsockopt`
pub(crate) fn sctp_getladdrs_internal(
    fd: RawFd,
    assoc_id: SctpAssociationId,
) -> std::io::Result<Vec<SocketAddr>> {
    sctp_getaddrs_internal(fd, SCTP_GET_LOCAL_ADDRS, assoc_id)
}

// Actual function performing `sctp_getpaddrs` or `sctp_getladdrs`
fn sctp_getaddrs_internal(
    fd: RawFd,
    flags: libc::c_int,
    assoc_id: SctpAssociationId,
) -> std::io::Result<Vec<SocketAddr>> {
    let capacity = 256_usize;
    let mut addrs_buff: Vec<u8> = vec![0; capacity];
    let mut getaddrs_size: libc::socklen_t = capacity as libc::socklen_t;

    // Safety: `addrs_buff` has a reserved capacity of 4K bytes which should normally be sufficient
    // for most of the calls to get local or peer addresses. Even if it is not sufficient, the call
    // to `getsockopt` would return an error, thus the memory won't be overwritten.
    unsafe {
        let mut getaddrs_ptr = addrs_buff.as_mut_ptr() as *mut SctpGetAddrs;
        eprintln!("getaddrs_ptr: {:?}", getaddrs_ptr);
        (*getaddrs_ptr).assoc_id = assoc_id;
        let getaddrs_size_ptr = std::ptr::addr_of_mut!(getaddrs_size);
        let result = libc::getsockopt(
            fd,
            SOL_SCTP,
            flags,
            getaddrs_ptr as *mut _ as *mut libc::c_void,
            getaddrs_size_ptr as *mut _ as *mut libc::socklen_t,
        );
        if result < 0 {
            eprintln!("result: {}", result);
            Err(std::io::Error::last_os_error())
        } else {
            let mut peeraddrs = vec![];

            // The call succeeded, we need to do a lot of ugly pointer arithmetic, first we get the
            // number of addresses of the peer `addr_count` written to by the call to `getsockopt`.
            let addr_count = (*getaddrs_ptr).addr_count;
            eprintln!("3:getaddrs: {:#?}", (*getaddrs_ptr));
            eprintln!("3:getaddrs: {:x?}", addrs_buff);

            let mut sockaddr_ptr = std::ptr::addr_of!((*getaddrs_ptr).addrs);
            for _ in 0..addr_count {
                // Now for each of the 'addresses', we try to get the family and then interpret
                // each of the addresses accordingly and update the pointer.
                let sa_family = (*(sockaddr_ptr as *const _ as *const libc::sockaddr)).sa_family;
                if sa_family as i32 == libc::AF_INET {
                    let os_socketaddr = OsSocketAddr::from_raw_parts(
                        sockaddr_ptr as *const _ as *const u8,
                        std::mem::size_of::<libc::sockaddr_in>(),
                    );
                    let socketaddr = os_socketaddr.into_addr().unwrap();
                    peeraddrs.push(socketaddr);
                    sockaddr_ptr = sockaddr_ptr
                        .offset(std::mem::size_of::<libc::sockaddr_in>().try_into().unwrap());
                } else if sa_family as i32 == libc::AF_INET6 {
                    let os_socketaddr = OsSocketAddr::from_raw_parts(
                        sockaddr_ptr as *const _ as *const u8,
                        std::mem::size_of::<libc::sockaddr_in6>(),
                    );
                    let socketaddr = os_socketaddr.into_addr().unwrap();
                    peeraddrs.push(socketaddr);
                    sockaddr_ptr = sockaddr_ptr.offset(
                        std::mem::size_of::<libc::sockaddr_in6>()
                            .try_into()
                            .unwrap(),
                    );
                } else {
                    // Unsupported Family - should never come here.
                    return Err(std::io::Error::from_raw_os_error(22));
                }
            }
            Ok(peeraddrs)
        }
    }
}

// Implementation of `sctp_connectx` using setsockopt.
pub(crate) fn sctp_connectx_internal(
    fd: RawFd,
    addrs: &[SocketAddr],
) -> std::io::Result<(SctpConnectedSocket, SctpAssociationId)> {
    let mut addrs_u8: Vec<u8> = vec![];

    for addr in addrs {
        let ossockaddr: OsSocketAddr = (*addr).into();
        let slice = ossockaddr.as_ref();
        addrs_u8.extend(slice);
    }

    let addrs_len = addrs_u8.len();

    // Safety: The passed vector is valid during the function call and hence the passed reference
    // to raw data is valid.
    unsafe {
        let result = libc::setsockopt(
            fd,
            SOL_SCTP,
            SCTP_SOCKOPT_CONNECTX,
            addrs_u8.as_ptr() as *const _ as *const libc::c_void,
            addrs_len as libc::socklen_t,
        );

        if result < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok((
                SctpConnectedSocket::from_rawfd(fd),
                result as SctpAssociationId,
            ))
        }
    }
}

// Implementation of `accept` - we just call the `libc::accept` allowing it to fail if the socket
// type is not the right one (UDP Style `SOCK_SEQPACKET`).
pub(crate) fn accept_internal(fd: RawFd) -> std::io::Result<(SctpConnectedSocket, SocketAddr)> {
    // this should be enough to `accept` a connection normally `sockaddr`s maximum size is 28 for
    // the `sa_family` we care about.
    let mut addrs_buff: Vec<u8> = vec![0; 32];
    addrs_buff.reserve(32);
    let mut addrs_len = addrs_buff.len();

    eprintln!("addrs_len: {}, addrs_u8: {:?}", addrs_len, addrs_buff,);
    // Safety: Both `addrs_buff` and `addrs_len` are in the scope and hence are valid pointers.
    unsafe {
        let addrs_len_ptr = std::ptr::addr_of_mut!(addrs_len);
        let addrs_buff_ptr = addrs_buff.as_mut_ptr();
        let result = libc::accept(
            fd,
            addrs_buff_ptr as *mut _ as *mut libc::sockaddr,
            addrs_len_ptr as *mut _ as *mut libc::socklen_t,
        );

        if result < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            let os_socketaddr = OsSocketAddr::from_raw_parts(addrs_buff.as_ptr(), addrs_len);
            eprintln!(
                "result: {},  addrs_len: {}, addrs_u8: {:?}",
                result, addrs_len, addrs_buff,
            );
            let socketaddr = os_socketaddr.into_addr().unwrap();
            Ok((SctpConnectedSocket::from_rawfd(result as RawFd), socketaddr))
        }
    }
}

// Shutdown implementation for `SctpListener` and `SctpConnectedSocket`.
pub(crate) fn shutdown_internal(fd: RawFd, how: std::net::Shutdown) -> std::io::Result<()> {
    use std::net::Shutdown;

    let flags = match how {
        Shutdown::Read => libc::SHUT_RD,
        Shutdown::Write => libc::SHUT_WR,
        Shutdown::Both => libc::SHUT_RDWR,
    };

    // Safety: No real undefined behavior as long as fd is a valid fd and if fd is not a valid fd
    // the underlying systemcall will error.
    unsafe {
        let result = libc::shutdown(fd, flags);
        if result < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

// Implementation for the receive side for SCTP.
// TODO: Handle Control Message Header
pub(crate) fn sctp_recvmsg_internal(fd: RawFd) -> std::io::Result<SctpNotificationOrData> {
    let mut recv_buffer = vec![0_u8; 4096];
    let mut recv_iov = libc::iovec {
        iov_base: recv_buffer.as_mut_ptr() as *mut _ as *mut libc::c_void,
        iov_len: recv_buffer.len(),
    };

    let mut from_buffer = vec![0u8; 256];
    let mut recvmsg_header = libc::msghdr {
        msg_name: from_buffer.as_mut_ptr() as *mut _ as *mut libc::c_void,
        msg_namelen: from_buffer.len() as u32,
        msg_iov: &mut recv_iov,
        msg_iovlen: 1,
        msg_control: std::ptr::null::<libc::c_int>() as *mut libc::c_void,
        msg_controllen: 0,
        msg_flags: 0,
    };

    // Safety: recvmsg_hdr is valid in the current scope.
    unsafe {
        let flags = 0 as libc::c_int;
        let result = libc::recvmsg(fd, &mut recvmsg_header as *mut libc::msghdr, flags);
        if result < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            let received_flags: u32 = recvmsg_header.msg_flags.try_into().unwrap();
            eprintln!("received_flags: {:x}", received_flags);
            recv_buffer.truncate(result as usize);
            eprintln!("buffer: {:x?}", recv_buffer);

            if received_flags & MSG_NOTIFICATION != 0 {
                Ok(SctpNotificationOrData::Notification(
                    notification_from_message(&recv_buffer),
                ))
            } else {
                Ok(SctpNotificationOrData::Data(recv_buffer))
            }
        }
    }
}

fn notification_from_message(data: &[u8]) -> SctpNotification {
    let notification_type = u16::from_ne_bytes(data[0..2].try_into().unwrap());
    eprintln!(
        "notification_type: {:x}, SCTP_ASSOC_CHANGE: {:x}",
        notification_type, SCTP_ASSOC_CHANGE
    );
    match notification_type {
        SCTP_ASSOC_CHANGE => {
            let assoc_change = AssociationChange {
                assoc_type: u16::from_ne_bytes(data[0..2].try_into().unwrap()),
                flags: u16::from_ne_bytes(data[2..4].try_into().unwrap()),
                length: u32::from_ne_bytes(data[4..8].try_into().unwrap()),
                state: u16::from_ne_bytes(data[8..10].try_into().unwrap()),
                error: u16::from_ne_bytes(data[10..12].try_into().unwrap()),
                ob_streams: u16::from_ne_bytes(data[12..14].try_into().unwrap()),
                ib_streams: u16::from_ne_bytes(data[14..16].try_into().unwrap()),
                assoc_id: i32::from_ne_bytes(data[16..20].try_into().unwrap()),
                info: data[20..].into(),
            };
            SctpNotification::AssociationChange(assoc_change)
        }
        _ => SctpNotification::Unsupported,
    }
}

// Implementation of Event Subscription
pub(crate) fn sctp_events_subscribe_internal(
    fd: RawFd,
    events: &[SctpEvent],
) -> std::io::Result<()> {
    let mut subscriber = SctpEventSubscribe::default();

    for ev in events {
        match ev {
            SctpEvent::DataIo => subscriber.data_io = 1,
            SctpEvent::Association => subscriber.association = 1,
            SctpEvent::Address => subscriber.address = 1,
            SctpEvent::SendFailure => subscriber.send_failure = 1,
            SctpEvent::PeerError => subscriber.peer_error = 1,
            SctpEvent::Shutdown => subscriber.shutdown = 1,
            SctpEvent::PartialDelivery => subscriber.partial_delivery = 1,
            SctpEvent::AdaptationLayer => subscriber.adaptation_layer = 1,
            SctpEvent::Authentication => subscriber.authentication = 1,
            SctpEvent::SenderDry => subscriber.sender_dry = 1,
            SctpEvent::StreamReset => subscriber.stream_reset = 1,
            SctpEvent::AssociationReset => subscriber.association_reset = 1,
            SctpEvent::StreamChange => subscriber.stream_change = 1,
            SctpEvent::SendFailureEvent => subscriber.send_failure_event = 1,
        }
    }

    eprintln!("subscriber: {:#?}", subscriber);
    // safety:
    unsafe {
        let result = libc::setsockopt(
            fd,
            SOL_SCTP,
            SCTP_EVENTS,
            &subscriber as *const _ as *const libc::c_void,
            std::mem::size_of::<SctpEventSubscribe>()
                .try_into()
                .unwrap(),
        );
        if result < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

// Close the socket
#[inline(always)]
pub(crate) fn close_internal(fd: RawFd) {
    unsafe {
        _ = libc::close(fd);
    }
}
