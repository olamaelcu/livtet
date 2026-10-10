//! Making things fail on purpose.
//!
//! Most of the uncovered lines left in this workspace are `.map_err` arms —
//! the branch taken when a query, an open, or a bind fails. They are not
//! reachable from a happy-path test, and each one otherwise needs its own
//! bespoke setup. These helpers are that setup, written once.

use camino::Utf8Path;

/// Rename `table` so the next query against it fails with "no such table".
///
/// Renaming rather than dropping: SQLite refuses to drop a parent table
/// whose children still hold rows, and dropping a table also drops the
/// triggers and indexes on it, which a later [`restore_table`] could not put
/// back. A rename is reversible and affects nothing else.
///
/// Takes a raw pool rather than a connection so it works for both fixture
/// families; the SQL is a fixed `ALTER TABLE` over a caller-supplied
/// identifier, which no test takes from untrusted input.
pub async fn hide_table(pool: &livtet_data::sql::SqlitePool, table: &str) {
    livtet_data::sql::query(livtet_data::sql::AssertSqlSafe(format!(
        "ALTER TABLE {table} RENAME TO {table}__hidden"
    )))
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("hide {table}: {e}"));
}

/// Undo [`hide_table`].
pub async fn restore_table(pool: &livtet_data::sql::SqlitePool, table: &str) {
    livtet_data::sql::query(livtet_data::sql::AssertSqlSafe(format!(
        "ALTER TABLE {table}__hidden RENAME TO {table}"
    )))
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("restore {table}: {e}"));
}

/// Reserve a TCP port that nothing is listening on.
///
/// Binds an ephemeral port, reads it back, and drops the listener. The port
/// is then free, which is what a test wants for two opposite reasons: to
/// hand a server a specific port to bind, or to point a client at an address
/// that will refuse the connection. Inherently a race — the OS may hand the
/// port to someone else before the caller uses it — but it is the only way
/// to learn a free port number, and it is what the existing server tests
/// already do.
pub fn reserve_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("read back the bound address")
        .port()
}

/// Make `path` unreadable, so opening it fails with `EACCES`.
///
/// Unix only: there is no portable equivalent, and the error arms this
/// reaches are the same on every platform. Returns the previous mode so the
/// caller can restore it — a temp directory whose contents cannot be read
/// also cannot be cleaned up.
#[cfg(unix)]
pub fn make_unreadable(path: &Utf8Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;

    let previous = fs_err::metadata(path)
        .unwrap_or_else(|e| panic!("stat {path}: {e}"))
        .permissions()
        .mode();
    fs_err::set_permissions(path, std::fs::Permissions::from_mode(0o000))
        .unwrap_or_else(|e| panic!("chmod {path}: {e}"));
    previous
}

/// Restore the mode [`make_unreadable`] returned.
#[cfg(unix)]
pub fn restore_mode(path: &Utf8Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;

    fs_err::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .unwrap_or_else(|e| panic!("chmod {path}: {e}"));
}

/// A path inside `dir` that does not exist and will not be created.
///
/// For the error arms that fire on a missing file, as distinct from one that
/// exists but cannot be read.
pub fn missing_path(dir: &Utf8Path, name: &str) -> camino::Utf8PathBuf {
    let path = dir.join(name);
    assert!(!path.exists(), "{path} was supposed to be absent");
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reserved_port_is_not_listening() {
        let port = reserve_port();
        assert!(port > 0);
        // Nothing is serving it, so connecting must be refused rather than
        // hanging or succeeding.
        assert!(
            std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
            "port {port} should be free"
        );
    }

    #[test]
    fn reserved_ports_differ() {
        // Not a guarantee the OS makes, but a collision would mean
        // `reserve_port` is returning a cached value rather than binding.
        let ports: Vec<u16> = (0..4).map(|_| reserve_port()).collect();
        let mut deduped = ports.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(deduped.len(), ports.len(), "got {ports:?}");
    }

    #[test]
    fn missing_path_is_absent_and_inside_the_directory() {
        let dir = camino_tempfile::tempdir().expect("tempdir");
        let path = missing_path(dir.path(), "nope.db");
        assert!(!path.exists());
        assert_eq!(path.parent(), Some(dir.path()));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_cannot_be_opened_and_the_mode_restores() {
        let dir = camino_tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("secret");
        fs_err::write(&path, b"contents").expect("write");
        assert!(fs_err::read(&path).is_ok(), "readable to begin with");

        let previous = make_unreadable(&path);
        // Root ignores permission bits, so only assert when we are not root.
        if !nix_is_root() {
            assert!(
                fs_err::read(&path).is_err(),
                "a 000 file must not be readable"
            );
        }

        restore_mode(&path, previous);
        assert!(fs_err::read(&path).is_ok(), "the mode must come back");
    }

    #[cfg(unix)]
    fn nix_is_root() -> bool {
        // SAFETY-free: `geteuid` is a pure syscall with no preconditions, but
        // the workspace denies unsafe, so shell out to the one safe source.
        std::env::var("USER").map(|u| u == "root").unwrap_or(false)
    }
}
