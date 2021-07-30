use tokio_postgres::{tls::MakeTlsConnect, types::Type, Client, Config, Connection, Error, Socket};

use std::sync::Arc;

#[derive(Clone)]
pub struct Database {
    inner: Arc<Client>, // TODO: WHy doesn't it implement clone?
}

impl Database {
    pub fn new(client: Client) -> Self {
        Self {
            inner: Arc::new(client),
        }
    }

    pub async fn connect<T>(
        config: &Config,
        tls: T,
    ) -> Result<(Self, Connection<Socket, T::Stream>), Error>
    where
        T: MakeTlsConnect<Socket>,
    {
        config
            .connect(tls)
            .await
            .map(|(client, connection)| (Self::new(client), connection))
    }

    pub async fn subscribe(&self, chat_id: i64, krate: &str) -> Result<(), Error> {
        let stmt = self
            .inner
            .prepare_typed("CALL subscribe($1, $2)", &[Type::INT8, Type::VARCHAR])
            .await?;

        self.inner.execute(&stmt, &[&chat_id, &krate]).await?;

        Ok(())
    }

    pub async fn unsubscribe(&self, chat_id: i64, krate: &str) -> Result<(), Error> {
        let stmt = self
            .inner
            .prepare_typed("CALL unsubscribe($1, $2)", &[Type::INT8, Type::VARCHAR])
            .await?;

        self.inner.execute(&stmt, &[&chat_id, &krate]).await?;

        Ok(())
    }

    pub async fn list_subscribers(&self, krate: &str) -> Result<impl Iterator<Item = i64>, Error> {
        let stmt = self
            .inner
            .prepare_typed("SELECT user_id from list_subscribers($1)", &[Type::VARCHAR])
            .await?;

        let res = self
            .inner
            .query(&stmt, &[&krate])
            .await?
            .into_iter()
            .map(|row| row.get(0));

        Ok(res)
    }

    pub async fn list_subscriptions(
        &self,
        chat_id: i64,
    ) -> Result<impl Iterator<Item = String>, Error> {
        let stmt = self
            .inner
            .prepare_typed(
                "SELECT crate_name from list_subscriptions($1)",
                &[Type::INT8],
            )
            .await?;

        let res = self
            .inner
            .query(&stmt, &[&chat_id])
            .await?
            .into_iter()
            .map(|row| row.get(0));

        Ok(res)
    }
}
