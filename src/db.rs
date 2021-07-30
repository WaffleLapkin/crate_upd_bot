use futures::Future;
use tokio_postgres::{
    tls::MakeTlsConnect, types::Type, Client, Config, Connection, Error, Socket, Statement,
};

use std::sync::Arc;

#[derive(Clone)]
pub struct Database {
    inner: Arc<Client>, // TODO: Why doesn't it implement clone?
    prepared: Arc<Prepared>,
}

impl Database {
    pub async fn connect<T>(
        config: &Config,
        tls: T,
    ) -> Result<(Self, Connection<Socket, T::Stream>), Error>
    where
        T: MakeTlsConnect<Socket>,
    {
        let (client, mut connection) = config.connect(tls).await?;

        let prepared = Prepared::prepare(&client, &mut connection).await?;

        let this = Self {
            inner: Arc::new(client),
            prepared: Arc::new(prepared),
        };

        Ok((this, connection))
    }

    pub async fn subscribe(&self, chat_id: i64, krate: &str) -> Result<(), Error> {
        let stmt = &self.prepared.subscribe;

        self.inner.execute(stmt, &[&chat_id, &krate]).await?;

        Ok(())
    }

    pub async fn unsubscribe(&self, chat_id: i64, krate: &str) -> Result<(), Error> {
        let stmt = &self.prepared.unsubscribe;

        self.inner.execute(stmt, &[&chat_id, &krate]).await?;

        Ok(())
    }

    pub async fn list_subscribers(&self, krate: &str) -> Result<impl Iterator<Item = i64>, Error> {
        let stmt = &self.prepared.list_subscribers;

        let res = self
            .inner
            .query(stmt, &[&krate])
            .await?
            .into_iter()
            .map(|row| row.get(0));

        Ok(res)
    }

    pub async fn list_subscriptions(
        &self,
        chat_id: i64,
    ) -> Result<impl Iterator<Item = String>, Error> {
        let stmt = &self.prepared.list_subscriptions;

        let res = self
            .inner
            .query(stmt, &[&chat_id])
            .await?
            .into_iter()
            .map(|row| row.get(0));

        Ok(res)
    }
}

struct Prepared {
    subscribe: Statement,
    unsubscribe: Statement,
    list_subscribers: Statement,
    list_subscriptions: Statement,
}

impl Prepared {
    async fn prepare(
        client: &Client,
        connection: &mut (impl Future<Output = Result<(), Error>> + Unpin),
    ) -> Result<Self, Error> {
        let prepare = async {
            let subscribe = client
                .prepare_typed("CALL subscribe($1, $2)", &[Type::INT8, Type::VARCHAR])
                .await?;

            let unsubscribe = client
                .prepare_typed("CALL unsubscribe($1, $2)", &[Type::INT8, Type::VARCHAR])
                .await?;

            let list_subscribers = client
                .prepare_typed("SELECT user_id from list_subscribers($1)", &[Type::VARCHAR])
                .await?;

            let list_subscriptions = client
                .prepare_typed(
                    "SELECT crate_name from list_subscriptions($1)",
                    &[Type::INT8],
                )
                .await?;

            Ok(Self {
                subscribe,
                unsubscribe,
                list_subscribers,
                list_subscriptions,
            })
        };

        tokio::select! {
            res = connection => {
                res?;
                unreachable!()
            },
            this = prepare => return this,
        }
    }
}
