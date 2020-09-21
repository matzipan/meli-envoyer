/*
 * meli - jmap module.
 *
 * Copyright 2019 Manos Pitsidianakis
 *
 * This file is part of meli.
 *
 * meli is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * meli is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli. If not, see <http://www.gnu.org/licenses/>.
 */

use super::mailbox::JmapMailbox;
use super::*;
use serde::Serialize;
use serde_json::{json, Value};
use std::convert::TryFrom;

pub type UtcDate = String;

use super::rfc8620::Object;

macro_rules! get_request_no {
    ($lock:expr) => {{
        let mut lck = $lock.lock().unwrap();
        let ret = *lck;
        *lck += 1;
        ret
    }};
}

pub trait Response<OBJ: Object> {
    const NAME: &'static str;
}

pub trait Method<OBJ: Object>: Serialize {
    const NAME: &'static str;
}

static USING: &[&str] = &["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    using: &'static [&'static str],
    /* Why is this Value instead of Box<dyn Method<_>>? The Method trait cannot be made into a
     * Trait object because its serialize() will be generic. */
    method_calls: Vec<Value>,

    #[serde(skip)]
    request_no: Arc<Mutex<usize>>,
}

impl Request {
    pub fn new(request_no: Arc<Mutex<usize>>) -> Self {
        Request {
            using: USING,
            method_calls: Vec::new(),
            request_no,
        }
    }

    pub fn add_call<M: Method<O>, O: Object>(&mut self, call: &M) -> usize {
        let seq = get_request_no!(self.request_no);
        self.method_calls
            .push(serde_json::to_value((M::NAME, call, &format!("m{}", seq))).unwrap());
        seq
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct JsonResponse<'a> {
    #[serde(borrow)]
    method_responses: Vec<MethodResponse<'a>>,
}

pub async fn get_mailboxes(conn: &JmapConnection) -> Result<HashMap<MailboxHash, JmapMailbox>> {
    let seq = get_request_no!(conn.request_no);
    let mut res = conn
        .client
        .post_async(
            &conn.session.api_url,
            serde_json::to_string(&json!({
                "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
                "methodCalls": [["Mailbox/get", {
                "accountId": conn.mail_account_id()
                },
                 format!("#m{}",seq).as_str()]],
            }))?,
        )
        .await?;

    let res_text = res.text_async().await?;
    let mut v: MethodResponse = serde_json::from_str(&res_text).unwrap();
    *conn.store.online_status.lock().await = (std::time::Instant::now(), Ok(()));
    let m = GetResponse::<MailboxObject>::try_from(v.method_responses.remove(0))?;
    let GetResponse::<MailboxObject> {
        list, account_id, ..
    } = m;
    *conn.store.account_id.lock().unwrap() = account_id;
    Ok(list
        .into_iter()
        .map(|r| {
            let MailboxObject {
                id,
                is_subscribed,
                my_rights,
                name,
                parent_id,
                role,
                sort_order,
                total_emails,
                total_threads,
                unread_emails,
                unread_threads,
            } = r;
            let hash = id.into_hash();
            let parent_hash = parent_id.clone().map(|id| id.into_hash());
            (
                hash,
                JmapMailbox {
                    name: name.clone(),
                    hash,
                    path: name,
                    children: Vec::new(),
                    id,
                    is_subscribed,
                    my_rights,
                    parent_id,
                    parent_hash,
                    role,
                    usage: Default::default(),
                    sort_order,
                    total_emails: Arc::new(Mutex::new(total_emails)),
                    total_threads,
                    unread_emails: Arc::new(Mutex::new(unread_emails)),
                    unread_threads,
                },
            )
        })
        .collect())
}

pub async fn get_message_list(
    conn: &JmapConnection,
    mailbox: &JmapMailbox,
) -> Result<Vec<Id<EmailObject>>> {
    let email_call: EmailQuery = EmailQuery::new(
        Query::new()
            .account_id(conn.mail_account_id().clone())
            .filter(Some(Filter::Condition(
                EmailFilterCondition::new()
                    .in_mailbox(Some(mailbox.id.clone()))
                    .into(),
            )))
            .position(0),
    )
    .collapse_threads(false);

    let mut req = Request::new(conn.request_no.clone());
    req.add_call(&email_call);

    let mut res = conn
        .client
        .post_async(&conn.session.api_url, serde_json::to_string(&req)?)
        .await?;

    let res_text = res.text_async().await?;
    let mut v: MethodResponse = serde_json::from_str(&res_text).unwrap();
    *conn.store.online_status.lock().await = (std::time::Instant::now(), Ok(()));
    let m = QueryResponse::<EmailObject>::try_from(v.method_responses.remove(0))?;
    let QueryResponse::<EmailObject> { ids, .. } = m;
    Ok(ids)
}

/*
pub async fn get_message(conn: &JmapConnection, ids: &[String]) -> Result<Vec<Envelope>> {
    let email_call: EmailGet = EmailGet::new(
        Get::new()
            .ids(Some(JmapArgument::value(ids.to_vec())))
            .account_id(conn.mail_account_id().to_string()),
    );

    let mut req = Request::new(conn.request_no.clone());
    req.add_call(&email_call);
    let mut res = conn
        .client
        .post_async(&conn.session.api_url, serde_json::to_string(&req)?)
        .await?;

    let res_text = res.text_async().await?;
    let mut v: MethodResponse = serde_json::from_str(&res_text).unwrap();
    let e = GetResponse::<EmailObject>::try_from(v.method_responses.remove(0))?;
    let GetResponse::<EmailObject> { list, .. } = e;
    Ok(list
        .into_iter()
        .map(std::convert::Into::into)
        .collect::<Vec<Envelope>>())
}
*/

pub async fn fetch(
    conn: &JmapConnection,
    store: &Store,
    mailbox_hash: MailboxHash,
) -> Result<Vec<Envelope>> {
    let mailbox_id = store.mailboxes.read().unwrap()[&mailbox_hash].id.clone();
    let email_query_call: EmailQuery = EmailQuery::new(
        Query::new()
            .account_id(conn.mail_account_id().clone())
            .filter(Some(Filter::Condition(
                EmailFilterCondition::new()
                    .in_mailbox(Some(mailbox_id))
                    .into(),
            )))
            .position(0),
    )
    .collapse_threads(false);

    let mut req = Request::new(conn.request_no.clone());
    let prev_seq = req.add_call(&email_query_call);

    let email_call: EmailGet = EmailGet::new(
        Get::new()
            .ids(Some(JmapArgument::reference(
                prev_seq,
                EmailQuery::RESULT_FIELD_IDS,
            )))
            .account_id(conn.mail_account_id().clone()),
    );

    req.add_call(&email_call);

    let mut res = conn
        .client
        .post_async(&conn.session.api_url, serde_json::to_string(&req)?)
        .await?;

    let res_text = res.text_async().await?;

    let mut v: MethodResponse = serde_json::from_str(&res_text).unwrap();
    let e = GetResponse::<EmailObject>::try_from(v.method_responses.pop().unwrap())?;
    let GetResponse::<EmailObject> { list, state, .. } = e;
    {
        let (is_empty, is_equal) = {
            let current_state_lck = conn.store.email_state.lock().unwrap();
            (current_state_lck.is_empty(), *current_state_lck != state)
        };
        if is_empty {
            debug!("{:?}: inserting state {}", EmailObject::NAME, &state);
            *conn.store.email_state.lock().unwrap() = state;
        } else if !is_equal {
            conn.email_changes().await?;
        }
    }
    let mut ret = Vec::with_capacity(list.len());
    for obj in list {
        ret.push(store.add_envelope(obj));
    }
    Ok(ret)
}

pub fn keywords_to_flags(keywords: Vec<String>) -> (Flag, Vec<String>) {
    let mut f = Flag::default();
    let mut tags = vec![];
    for k in keywords {
        match k.as_str() {
            "$draft" => {
                f |= Flag::DRAFT;
            }
            "$seen" => {
                f |= Flag::SEEN;
            }
            "$flagged" => {
                f |= Flag::FLAGGED;
            }
            "$answered" => {
                f |= Flag::REPLIED;
            }
            "$junk" | "$notjunk" => { /* ignore */ }
            _ => tags.push(k),
        }
    }
    (f, tags)
}
