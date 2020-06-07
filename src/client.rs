use std::convert::TryFrom;
use std::net::ToSocketAddrs;
use std::time::Duration;

use log::*;

use crate::error::{Error, Result};
use crate::raw::connection::{NntpConnection, TlsConfig};

use crate::types::command as cmd;
use crate::types::prelude::*;

/// A client
///
/// Each client represents a single connection
#[derive(Debug)]
pub struct NntpClient {
    conn: NntpConnection,
    config: ClientConfig,
    capabilities: Capabilities,
    group: Option<Group>,
}

impl NntpClient {
    /// Get the raw [`NntpConnection`] for the client
    fn conn(&mut self) -> &mut NntpConnection {
        &mut self.conn
    }

    /// Get the currently selected group
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    /// Get the last selected group
    pub fn group(&self) -> Option<&Group> {
        self.group.as_ref()
    }

    pub fn set_group(&mut self, name: impl AsRef<str>) -> Result<Group> {
        let resp = self.conn.command(&cmd::Group(name.as_ref().to_string()))?;

        match resp.code() {
            ResponseCode::Known(Kind::GroupSelected) => {
                let group = Group::try_from(&resp)?;
                self.group = Some(group.clone());
                Ok(group)
            }
            ResponseCode::Known(Kind::NoSuchNewsgroup) => Err(Error::bad_response(resp)),
            code => Err(Error::Failure {
                code,
                msg: Some(format!("{}", resp.first_line_to_utf8_lossy())),
                resp,
            }),
        }
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn update_capabilities(&mut self) -> Result<&Capabilities> {
        let resp = self.conn.command(&cmd::Capabilities)?;
        if resp.code() != ResponseCode::Known(Kind::Capabilities) {
            return Err(Error::bad_response(resp));
        }
        let capabilities = Capabilities::try_from(&resp)?;

        self.capabilities = capabilities;

        Ok(&self.capabilities)
    }

    /// FIXME(docs)
    ///
    /// # Implementation Notes
    ///
    /// * This client does not properly implement "header folding" for text
    /// * Netnews articles containing non-utf8 characters MUST be binary
    fn article(&mut self, _article: cmd::Article) -> Result<()> {
        unimplemented!()
    }

    fn overviews(&mut self, _overview: cmd::Over) -> Result<()> {
        // check capabilities for over and xover
        unimplemented!()
    }

    fn list(&mut self, _list: cmd::List) -> Result<()> {
        unimplemented!()
    }

    /// Close the connection to the server
    pub fn close(&mut self) -> Result<()> {
        let resp = self.conn.command(&cmd::Quit)?;

        if resp.code != ResponseCode::Known(Kind::ConnectionClosing) {
            Err(Error::Failure {
                code: resp.code,
                resp,
                msg: Some("Failed to close connection".to_string()),
            })
        } else {
            self.group = None;
            // TODO: return response from server
            Ok(())
        }
    }
}

// TODO: Derive Debug once https://github.com/sfackler/rust-native-tls/issues/99 is implemented
/// Configuration for an [`NntpClient`]
#[derive(Clone, Debug)]
pub struct ClientConfig {
    tls_config: Option<TlsConfig>,
    authinfo: Option<(String, String)>,
    group: Option<String>,
    read_timeout: Option<Duration>,
}

impl ClientConfig {
    pub fn new() -> Self {
        ClientConfig {
            tls_config: None,
            authinfo: None,
            group: None,
            read_timeout: None,
        }
    }
    /// Perform an AUTHINFO USER/PASS authentication after connecting to the server
    ///
    /// https://tools.ietf.org/html/rfc4643#section-2.3
    pub fn authinfo_user_pass(&mut self, username: String, password: String) -> &mut Self {
        self.authinfo = Some((username, password));
        self
    }

    pub fn tls_config(&mut self, config: TlsConfig) -> &mut Self {
        self.tls_config = Some(config);
        self
    }

    pub fn default_tls(&mut self, domain: String) -> Result<&mut Self> {
        self.tls_config = Some(TlsConfig::default_connector(domain)?);
        Ok(self)
    }

    pub fn group(&mut self, name: String) -> &mut Self {
        self.group = Some(name);
        self
    }

    pub fn read_timeout(&mut self, duration: Option<Duration>) -> &mut Self {
        self.read_timeout = duration;
        self
    }

    // FIXME(ux): Add timeout support

    /// Resolves the configuration into a client
    pub fn connect(&self, addr: impl ToSocketAddrs) -> Result<NntpClient> {
        let (mut conn, conn_response) =
            NntpConnection::connect(addr, self.tls_config.clone(), self.read_timeout)?;

        debug!(
            "Connected. Server returned `{}`",
            conn_response.first_line_to_utf8_lossy()
        );

        // FIXME(correctness) check capabilities before attempting auth info
        if let Some((username, password)) = &self.authinfo {
            if self.tls_config.is_none() {
                warn!("TLS is not enabled, credentials will be sent in the clear!");
            }
            debug!("Authenticating with AUTHINFO USER/PASS");
            authenticate(&mut conn, username, password)?;
        }

        debug!("Retrieving capabilities...");
        let capabilities = get_capabilities(&mut conn)?;

        let group = if let Some(name) = &self.group {
            debug!("Connecting to group {}...", name);
            select_group(&mut conn, name)?.into()
        } else {
            debug!("No initial group specified");
            None
        };

        Ok(NntpClient {
            conn,
            config: self.clone(),
            capabilities,
            group,
        })
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            tls_config: None,
            authinfo: None,
            group: None,
            read_timeout: None,
        }
    }
}

/// Perform an AUTHINFO USER/PASS exchange
fn authenticate(
    conn: &mut NntpConnection,
    username: impl AsRef<str>,
    password: impl AsRef<str>,
) -> Result<()> {
    debug!("Sending AUTHINFO USER");
    let user_resp = conn.command(&cmd::AuthInfo::User(username.as_ref().to_string()))?;

    if user_resp.code != ResponseCode::from(381) {
        return Err(Error::Failure {
            code: user_resp.code,
            resp: user_resp,
            msg: Some("AUTHINFO USER failed".to_string()),
        });
    }

    debug!("Sending AUTHINFO PASS");
    let pass_resp = conn.command(&cmd::AuthInfo::Pass(password.as_ref().to_string()))?;

    if pass_resp.code() != ResponseCode::Known(Kind::AuthenticationAccepted) {
        return Err(Error::Failure {
            code: pass_resp.code,
            resp: pass_resp,
            msg: Some("AUTHINFO PASS failed".to_string()),
        });
    }
    debug!("Successfully authenticated");

    Ok(())
}

fn get_capabilities(conn: &mut NntpConnection) -> Result<Capabilities> {
    let resp = conn.command(&cmd::Capabilities)?;

    if resp.code() != ResponseCode::Known(Kind::Capabilities) {
        Err(Error::bad_response(resp))
    } else {
        Capabilities::try_from(&resp)
    }
}

fn select_group(conn: &mut NntpConnection, group: impl AsRef<str>) -> Result<Group> {
    let resp = conn.command(&cmd::Group(group.as_ref().to_string()))?;

    match resp.code() {
        ResponseCode::Known(Kind::GroupSelected) => Group::try_from(&resp),
        ResponseCode::Known(Kind::NoSuchNewsgroup) => Err(Error::bad_response(resp)),
        code => Err(Error::Failure {
            code,
            msg: Some(format!("{}", resp.first_line_to_utf8_lossy())),
            resp,
        }),
    }
}
