//! Kernel ingress age for the shared P2 acquisition socket. Buffering must not
//! make a queued datagram appear to have arrived when userspace finally reads it.
//! RX_SOFTWARE + SOFTWARE avoids SO_TIMESTAMP[NS] dequeue fallback timestamps;
//! missing network-stack evidence is rejected, never replaced with now().
use std::{
    io, mem,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket},
    os::fd::AsRawFd,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const RECEIVE_BYTES: libc::c_int = 512 * 1024;
const CLOCK_UNCERTAINTY: Duration = Duration::from_millis(5);

pub(super) type Clock = (Instant, SystemTime);

pub(super) fn enable(socket: &UdpSocket) -> io::Result<Clock> {
    let clock = (Instant::now(), SystemTime::now());
    let fd = socket.as_raw_fd();
    for (option, value) in [
        (
            libc::SO_TIMESTAMPING,
            (libc::SOF_TIMESTAMPING_RX_SOFTWARE | libc::SOF_TIMESTAMPING_SOFTWARE) as libc::c_int,
        ),
        (libc::SO_RCVBUF, RECEIVE_BYTES),
    ] {
        let result = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                option,
                (&value as *const libc::c_int).cast(),
                mem::size_of_val(&value) as _,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let mut effective: libc::c_int = 0;
    let mut length = mem::size_of_val(&effective) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&mut effective as *mut libc::c_int).cast(),
            &mut length,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    println!("saturn-bridge: shared P2 kernel ingress timestamps enabled SO_RCVBUF requested={RECEIVE_BYTES} effective={effective}; queued age retained; freshness limits unchanged");
    // Linux enables receive timestamp generation asynchronously. Wait for an
    // actual timestamp on a PRIVATE loopback socket before radio bootstrap;
    // never accept an unstamped first radio packet or sleep for an assumed delay.
    let probe = UdpSocket::bind("127.0.0.1:0")?;
    probe.set_read_timeout(Some(Duration::from_millis(50)))?;
    let flags =
        (libc::SOF_TIMESTAMPING_RX_SOFTWARE | libc::SOF_TIMESTAMPING_SOFTWARE) as libc::c_int;
    if unsafe {
        libc::setsockopt(
            probe.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_TIMESTAMPING,
            (&flags as *const libc::c_int).cast(),
            mem::size_of_val(&flags) as _,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let end = Instant::now() + Duration::from_secs(2);
    loop {
        probe.send_to(b"private-ingress-clock", probe.local_addr()?)?;
        if receive(&probe, &mut [0; 64], clock).is_ok() {
            break;
        }
        if Instant::now() >= end {
            return Err(invalid("P2 network-stack ingress timestamps unavailable"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(clock)
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

fn arrival(
    stamp: SystemTime,
    before: Clock,
    after: (Instant, SystemTime, Instant),
) -> io::Result<Instant> {
    let elapsed = after
        .0
        .checked_duration_since(before.0)
        .ok_or_else(|| invalid("P2 ingress monotonic clock reversed"))?;
    let wall_elapsed = after
        .1
        .duration_since(before.1)
        .map_err(|_| invalid("P2 ingress wall clock reversed"))?;
    let uncertainty = after
        .2
        .checked_duration_since(after.0)
        .ok_or_else(|| invalid("P2 ingress clock sampling reversed"))?;
    if elapsed.abs_diff(wall_elapsed) > CLOCK_UNCERTAINTY || uncertainty > CLOCK_UNCERTAINTY {
        return Err(invalid("P2 kernel ingress clock mapping uncertain"));
    }
    let age = after
        .1
        .duration_since(stamp)
        .map_err(|_| invalid("P2 kernel ingress timestamp is in the future"))?;
    // The monotonic anchor precedes the wall-clock sample: any sampling skew
    // conservatively makes the observation older, never fresher. Subtract the
    // allowed clock-mapping uncertainty too; it cannot buy extra freshness.
    after
        .0
        .checked_sub(age + CLOCK_UNCERTAINTY)
        .ok_or_else(|| invalid("P2 kernel ingress timestamp outside monotonic range"))
}

pub(super) fn receive(
    socket: &UdpSocket,
    bytes: &mut [u8],
    clock: Clock,
) -> io::Result<(usize, SocketAddr, Instant)> {
    let mut address: libc::sockaddr_storage = unsafe { mem::zeroed() };
    let mut control = [0usize; 16]; // aligned ancillary storage, no allocation
    let mut iov = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let mut message: libc::msghdr = unsafe { mem::zeroed() };
    message.msg_name = (&mut address as *mut libc::sockaddr_storage).cast();
    message.msg_namelen = mem::size_of_val(&address) as _;
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = mem::size_of_val(&control);
    let size = unsafe { libc::recvmsg(socket.as_raw_fd(), &mut message, 0) };
    if size < 0 {
        return Err(io::Error::last_os_error());
    }
    let after = (Instant::now(), SystemTime::now(), Instant::now());
    if message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0 {
        return Err(invalid("P2 truncated datagram or ingress timestamp"));
    }
    let mut timestamp = None;
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&message);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SO_TIMESTAMPING
            {
                if (*cmsg).cmsg_len
                    < libc::CMSG_LEN((3 * mem::size_of::<libc::timespec>()) as _) as _
                {
                    return Err(invalid("P2 short kernel ingress timestamp"));
                }
                let value = libc::CMSG_DATA(cmsg)
                    .cast::<libc::timespec>()
                    .read_unaligned();
                if value.tv_sec <= 0 || !(0..1_000_000_000).contains(&value.tv_nsec) {
                    return Err(invalid("P2 invalid kernel ingress timestamp"));
                }
                timestamp = UNIX_EPOCH
                    .checked_add(Duration::new(value.tv_sec as u64, value.tv_nsec as u32));
            }
            cmsg = libc::CMSG_NXTHDR(&message, cmsg);
        }
    }
    let source = unsafe {
        match address.ss_family as libc::c_int {
            libc::AF_INET => {
                let a = &*(&address as *const libc::sockaddr_storage).cast::<libc::sockaddr_in>();
                SocketAddr::from((
                    Ipv4Addr::from(a.sin_addr.s_addr.to_ne_bytes()),
                    u16::from_be(a.sin_port),
                ))
            }
            libc::AF_INET6 => {
                let a = &*(&address as *const libc::sockaddr_storage).cast::<libc::sockaddr_in6>();
                SocketAddr::V6(std::net::SocketAddrV6::new(
                    Ipv6Addr::from(a.sin6_addr.s6_addr),
                    u16::from_be(a.sin6_port),
                    a.sin6_flowinfo,
                    a.sin6_scope_id,
                ))
            }
            _ => return Err(invalid("P2 ingress address family unavailable")),
        }
    };
    let observed = arrival(
        timestamp.ok_or_else(|| {
            invalid("P2 kernel ingress timestamp missing; no userspace retiming fallback")
        })?,
        clock,
        after,
    )?;
    Ok((size as usize, source, observed))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_kernel_datagram_keeps_original_age() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        let clock = enable(&receiver).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender
            .send_to(b"p2-original", receiver.local_addr().unwrap())
            .unwrap();
        std::thread::sleep(Duration::from_millis(180));
        let (n, source, at) = receive(&receiver, &mut [0; 64], clock).unwrap();
        assert_eq!(n, 11);
        assert_eq!(source, sender.local_addr().unwrap());
        assert!(
            at.elapsed() >= Duration::from_millis(150),
            "Expired queued evidence must stay expired: elapsed={:?}",
            at.elapsed()
        );
    }
    #[test]
    fn missing_kernel_timestamp_never_becomes_fresh() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        let clock = (Instant::now(), SystemTime::now());
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender
            .send_to(b"p2", receiver.local_addr().unwrap())
            .unwrap();
        assert_eq!(
            receive(&receiver, &mut [0; 64], clock).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[test]
    fn truncated_datagram_is_not_admitted() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        let clock = enable(&receiver).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender
            .send_to(&[0; 1444], receiver.local_addr().unwrap())
            .unwrap();
        assert_eq!(
            receive(&receiver, &mut [0; 64], clock).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[test]
    fn clock_steps_uncertainty_and_future_timestamps_are_rejected() {
        let mono = Instant::now();
        let wall = SystemTime::now();
        let before = (mono, wall);
        let after = (
            mono + Duration::from_millis(20),
            wall + Duration::from_millis(20),
            mono + Duration::from_millis(20),
        );
        assert_eq!(
            arrival(wall, before, after).unwrap(),
            mono - CLOCK_UNCERTAINTY
        );
        assert!(arrival(
            wall,
            before,
            (after.0, wall + Duration::from_secs(1), after.2)
        )
        .is_err());
        assert!(arrival(
            wall,
            before,
            (after.0, wall - Duration::from_secs(1), after.2)
        )
        .is_err());
        assert!(arrival(wall + Duration::from_secs(1), before, after).is_err());
        assert!(arrival(
            wall,
            before,
            (after.0, after.1, after.2 + Duration::from_millis(10))
        )
        .is_err());
    }
}
