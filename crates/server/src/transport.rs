//! HTTP transport over Unix sockets or owner-restricted Windows named pipes.

use hoista_system::paths::StatePaths;
use interprocess::local_socket::{
    ListenerOptions, Name,
    tokio::{Listener, Stream},
    traits::tokio::{Listener as _, Stream as _},
};
use std::{io, time::Duration};

/// The wait before accepting again after a failed accept.
const ACCEPT_RETRY: Duration = Duration::from_millis(250);

pub(crate) fn endpoint(paths: &StatePaths) -> io::Result<Name<'static>> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::{GenericFilePath, ToFsName};
        Ok(paths.socket().to_fs_name::<GenericFilePath>()?.into_owned())
    }

    #[cfg(windows)]
    {
        use interprocess::local_socket::{GenericNamespaced, ToNsName};
        use sha2::{Digest, Sha256};
        let canonical = std::fs::canonicalize(paths.root())?;
        let hash = Sha256::digest(canonical.to_string_lossy().as_bytes());
        Ok(format!("hoista-{hash:x}")
            .to_ns_name::<GenericNamespaced>()?
            .into_owned())
    }
}

pub(crate) struct LocalListener(Listener);

impl LocalListener {
    pub(crate) fn bind(paths: &StatePaths) -> io::Result<Self> {
        // The private parent protects the socket even on systems without pre-bind fchmod.
        paths.prepare()?;
        let options = ListenerOptions::new()
            .name(endpoint(paths)?)
            .reclaim_name(true);
        #[cfg(all(unix, not(target_os = "macos")))]
        let options = {
            use interprocess::os::unix::local_socket::ListenerOptionsExt;
            options.mode(0o600)
        };
        #[cfg(windows)]
        let options = {
            use interprocess::os::windows::{
                local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
            };
            // OW is the owner-rights SID; no Everyone, Users or network grants.
            options.security_descriptor(SecurityDescriptor::deserialize(widestring::u16cstr!(
                "D:P(A;;GA;;;OW)"
            ))?)
        };
        let listener = options.create_tokio()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // macOS rejects fchmod on an unbound socket. The enclosing 0700 directory
            // prevents other users from connecting during this post-bind chmod.
            std::fs::set_permissions(paths.socket(), std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(Self(listener))
    }
}

impl axum::serve::Listener for LocalListener {
    type Io = Stream;
    type Addr = ();

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.0.accept().await {
                Ok(stream) => return (stream, ()),
                Err(_) => {
                    tracing::error!(code = "socket.accept_failed");
                    tokio::time::sleep(ACCEPT_RETRY).await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(())
    }
}

pub(crate) async fn connect(paths: &StatePaths) -> io::Result<Stream> {
    Stream::connect(endpoint(paths)?).await
}
