use crate::client::Client;
use crate::document::{Document, DocumentCollection};
use crate::error::CouchError;
use crate::types::design::DesignCreated;
use crate::types::document::{DocumentCreatedResult, DocumentId};
use crate::types::find::{FindQuery, FindResult};
use crate::types::index::{DatabaseIndexList, IndexFields};
use crate::types::query::{QueriesCollection, QueriesParams, QueryParams};
use crate::types::view::ViewCollection;
use reqwest::{RequestBuilder, StatusCode};
use serde_json::{json, to_string, Value};
use std::collections::HashMap;
use tokio::sync::mpsc::Sender;

/// Database operations on a CouchDB Database
/// (sometimes called Collection in other NoSQL flavors such as MongoDB).
#[derive(Debug, Clone)]
pub struct Database {
    _client: Client,
    name: String,
}

impl Database {
    pub fn new(name: String, client: Client) -> Database {
        Database { _client: client, name }
    }

    // convenience function to retrieve the name of the database
    pub fn name(&self) -> &str {
        &self.name
    }

    fn create_document_path(&self, id: &str) -> String {
        let mut result: String = self.name.clone();
        result.push_str("/");
        result.push_str(id);
        result
    }

    fn create_design_path(&self, id: &str) -> String {
        let mut result: String = self.name.clone();
        result.push_str("/_design/");
        result.push_str(id);
        result
    }

    fn create_query_view_path(&self, design_id: &str, view_id: &str) -> String {
        let mut result: String = self.name.clone();
        result.push_str("/_design/");
        result.push_str(design_id);
        result.push_str("/_view/");
        result.push_str(view_id);
        result
    }

    fn create_execute_update_path(&self, design_id: &str, update_id: &str, document_id: &str) -> String {
        let mut result: String = self.name.clone();
        result.push_str("/_design/");
        result.push_str(design_id);
        result.push_str("/_update/");
        result.push_str(update_id);
        result.push_str("/");
        result.push_str(document_id);
        result
    }

    fn create_compact_path(&self, design_name: &str) -> String {
        let mut result: String = self.name.clone();
        result.push_str("/_compact/");
        result.push_str(design_name);
        result
    }

    async fn is_accepted(&self, request: Result<RequestBuilder, CouchError>) -> bool {
        if let Ok(req) = request {
            if let Ok(res) = req.send().await {
                return res.status() == StatusCode::ACCEPTED;
            }
        }

        false
    }

    async fn is_ok(&self, request: Result<RequestBuilder, CouchError>) -> bool {
        if let Ok(req) = request {
            if let Ok(res) = req.send().await {
                return match res.status() {
                    StatusCode::OK | StatusCode::NOT_MODIFIED => true,
                    _ => false,
                };
            }
        }

        false
    }

    /// Launches the compact process
    pub async fn compact(&self) -> bool {
        let mut path: String = self.name.clone();
        path.push_str("/_compact");

        let request = self._client.post(path, "".into());
        self.is_accepted(request).await
    }

    /// Starts the compaction of all views
    pub async fn compact_views(&self) -> bool {
        let mut path: String = self.name.clone();
        path.push_str("/_view_cleanup");

        let request = self._client.post(path, "".into());
        self.is_accepted(request).await
    }

    /// Starts the compaction of a given index
    pub async fn compact_index(&self, index: &str) -> bool {
        let request = self._client.post(self.create_compact_path(index), "".into());
        self.is_accepted(request).await
    }

    /// Checks if a document ID exists
    ///
    /// Usage:
    /// ```
    /// use std::error::Error;
    ///
    /// const DB_HOST: &str = "http://admin:password@localhost:5984";
    /// const TEST_DB: &str = "test_db";
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let client = couch_rs::Client::new(DB_HOST)?;
    ///     let db = client.db(TEST_DB).await?;
    ///
    ///     // check if the design document "_design/clip_view" exists
    ///     if db.exists("_design/clip_view").await {
    ///         println!("The design document exists");
    ///     }   
    ///
    ///     return Ok(());
    /// }
    /// ```
    pub async fn exists(&self, id: &str) -> bool {
        let request = self._client.head(self.create_document_path(id), None);
        self.is_ok(request).await
    }

    /// Gets one document
    pub async fn get(&self, id: &str) -> Result<Document, CouchError> {
        let response = self
            ._client
            .get(self.create_document_path(id), None)?
            .send()
            .await?
            .error_for_status()?;
        Ok(Document::new(response.json().await?))
    }

    /// Gets documents in bulk with provided IDs list
    pub async fn get_bulk(&self, ids: Vec<DocumentId>) -> Result<DocumentCollection, CouchError> {
        self.get_bulk_params(ids, None).await
    }

    /// Each time a document is stored or updated in CouchDB, the internal B-tree is updated.
    /// Bulk insertion provides efficiency gains in both storage space, and time,
    /// by consolidating many of the updates to intermediate B-tree nodes.
    ///
    /// See the documentation on how to use bulk_docs here: [db-bulk-docs](https://docs.couchdb.org/en/stable/api/database/bulk-api.html#db-bulk-docs)
    ///
    /// raw_docs is a vector of documents with or without an ID
    ///
    /// This endpoint can also be used to delete a set of documents by including "_deleted": true, in the document to be deleted.
    /// When deleting or updating, both _id and _rev are mandatory.
    pub async fn bulk_docs(&self, raw_docs: Vec<Value>) -> Result<Vec<DocumentCreatedResult>, CouchError> {
        let mut body = HashMap::new();
        body.insert(s!("docs"), raw_docs);

        let response = self
            ._client
            .post(self.create_document_path("_bulk_docs"), to_string(&body)?)?
            .send()
            .await?;

        let data: Vec<DocumentCreatedResult> = response.json().await?;

        Ok(data)
    }

    /// Gets documents in bulk with provided IDs list, with added params. Params description can be found here:
    /// [_all_docs](https://docs.couchdb.org/en/latest/api/database/bulk-api.html?highlight=_all_docs)
    ///
    /// Usage:
    ///
    /// ```
    /// use couch_rs::types::find::FindQuery;
    /// use couch_rs::document::Document;
    /// use std::error::Error;
    /// use serde_json::json;
    ///
    /// const DB_HOST: &str = "http://admin:password@localhost:5984";
    /// const TEST_DB: &str = "test_db";
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let client = couch_rs::Client::new(DB_HOST)?;
    ///     let db = client.db(TEST_DB).await?;
    ///     let doc_1 = Document::new(json!({
    ///                     "_id": "john",
    ///                     "first_name": "John",
    ///                     "last_name": "Doe"
    ///                 }));
    ///
    ///     let doc_2 = Document::new(json!({
    ///                     "_id": "jane",
    ///                     "first_name": "Jane",
    ///                     "last_name": "Doe"
    ///                 }));
    ///
    ///     // Save these documents
    ///     db.save(doc_1).await?;
    ///     db.save(doc_2).await?;
    ///
    ///     // subsequent call updates the existing document
    ///     let docs = db.get_bulk_params(vec!["john".to_string(), "jane".to_string()], None).await?;
    ///
    ///     // verify that we received the 2 documents
    ///     assert_eq!(docs.rows.len(), 2);
    ///     Ok(())
    /// }
    /// ```   
    pub async fn get_bulk_params(
        &self,
        ids: Vec<DocumentId>,
        params: Option<QueryParams>,
    ) -> Result<DocumentCollection, CouchError> {
        let mut options;
        if let Some(opts) = params {
            options = opts;
        } else {
            options = QueryParams::default();
        }

        options.include_docs = Some(true);

        let mut body = HashMap::new();
        body.insert(s!("keys"), ids);

        let response = self
            ._client
            .post(self.create_document_path("_all_docs"), to_string(&body)?)?
            .query(&options)
            .send()
            .await?
            .error_for_status()?;

        Ok(DocumentCollection::new(response.json().await?))
    }

    /// Gets all the documents in database
    pub async fn get_all(&self) -> Result<DocumentCollection, CouchError> {
        self.get_all_params(None).await
    }

    /// Gets all documents in the database, using bookmarks to iterate through all the documents.
    /// Results are returned through an mpcs channel for async processing. Use this for very large
    /// databases only. Batch size can be requested. A value of 0, means the default batch_size of
    /// 1000 is used. max_results of 0 means all documents will be returned. A given max_results is
    /// always rounded *up* to the nearest multiplication of batch_size.
    /// This operation is identical to find_batched(FindQuery::find_all(), tx, batch_size, max_results)
    ///
    /// Check out the async_batch_read example for usage details
    pub async fn get_all_batched(
        &self,
        tx: Sender<DocumentCollection>,
        batch_size: u64,
        max_results: u64,
    ) -> Result<u64, CouchError> {
        let query = FindQuery::find_all();
        self.find_batched(query, tx, batch_size, max_results).await
    }

    /// Finds documents in the database, using bookmarks to iterate through all the documents.
    /// Results are returned through an mpcs channel for async processing. Use this for very large
    /// databases only. Batch size can be requested. A value of 0, means the default batch_size of
    /// 1000 is used. max_results of 0 means all documents will be returned. A given max_results is
    /// always rounded *up* to the nearest multiplication of batch_size.
    ///
    /// Check out the async_batch_read example for usage details
    pub async fn find_batched(
        &self,
        mut query: FindQuery,
        mut tx: Sender<DocumentCollection>,
        batch_size: u64,
        max_results: u64,
    ) -> Result<u64, CouchError> {
        let mut bookmark = Option::None;
        let limit = if batch_size > 0 { batch_size } else { 1000 };

        let mut results: u64 = 0;
        query.limit = Option::Some(limit);

        let maybe_err = loop {
            let mut segment_query = query.clone();
            segment_query.bookmark = bookmark.clone();
            let all_docs = match self.find(&query).await {
                Ok(docs) => docs,
                Err(err) => break Some(err),
            };

            if all_docs.total_rows == 0 {
                // no more rows
                break None;
            }

            if all_docs.bookmark.is_some() && all_docs.bookmark != bookmark {
                bookmark.replace(all_docs.bookmark.clone().unwrap_or_default());
            } else {
                // no bookmark, break the query loop
                break None;
            }

            results += all_docs.total_rows as u64;

            tx.send(all_docs).await.unwrap();

            if max_results > 0 && results >= max_results {
                break None;
            }
        };

        if let Some(err) = maybe_err {
            Err(err)
        } else {
            Ok(results)
        }
    }

    /// Executes multiple specified built-in view queries of all documents in this database.
    /// This enables you to request multiple queries in a single request, in place of multiple POST /{db}/_all_docs requests.
    /// [More information](https://docs.couchdb.org/en/stable/api/database/bulk-api.html#sending-multiple-queries-to-a-database)
    /// Parameters description can be found [here](https://docs.couchdb.org/en/latest/api/ddoc/views.html#api-ddoc-view)
    ///
    /// Usage:
    /// ```
    /// use couch_rs::types::find::FindQuery;
    /// use couch_rs::types::query::{QueryParams, QueriesParams};
    /// use couch_rs::document::Document;
    /// use std::error::Error;
    /// use serde_json::json;
    ///
    /// const DB_HOST: &str = "http://admin:password@localhost:5984";
    /// const TEST_DB: &str = "test_db";
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let client = couch_rs::Client::new(DB_HOST)?;
    ///     let db = client.db(TEST_DB).await?;
    ///
    ///     // imagine we have a database (e.g. vehicles) with multiple documents of different types; e.g. cars, planes and boats
    ///     // document IDs have been generated taking this into account, so cars have IDs starting with "car:",
    ///     // planes have IDs starting with "plane:", and boats have IDs starting with "boat:"
    ///     //
    ///     // let's query for all cars and all boats, sending just 1 request
    ///     let mut cars = QueryParams::default();
    ///     cars.start_key = Some("car".to_string());
    ///     cars.end_key = Some("car:\u{fff0}".to_string());
    ///
    ///     let mut boats = QueryParams::default();
    ///     boats.start_key = Some("boat".to_string());
    ///     boats.end_key = Some("boat:\u{fff0}".to_string());
    ///
    ///     let mut collections = db.query_many_all_docs(QueriesParams::new(vec![cars, boats])).await?;
    ///     println!("Succeeded querying for cars and boats");
    ///     let mut collections = collections.iter_mut();
    ///     let car_collection = collections.next().unwrap();
    ///     println!("Retrieved cars {:?}", car_collection);
    ///     let boat_collection = collections.next().unwrap();
    ///     println!("Retrieved boats {:?}", boat_collection);
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn query_many_all_docs(&self, queries: QueriesParams) -> Result<Vec<ViewCollection>, CouchError> {
        self.query_view_many(self.create_document_path("_all_docs/queries"), queries)
            .await
    }

    /// Executes multiple queries against a view.
    pub async fn query_many(
        &self,
        design_name: &str,
        view_name: &str,
        queries: QueriesParams,
    ) -> Result<Vec<ViewCollection>, CouchError> {
        self.query_view_many(self.create_query_view_path(design_name, view_name), queries)
            .await
    }

    async fn query_view_many(
        &self,
        view_path: String,
        queries: QueriesParams,
    ) -> Result<Vec<ViewCollection>, CouchError> {
        // we use POST here, because this allows for a larger set of keys to be provided, compared
        // to a GET call. It provides the same functionality

        let response = self
            ._client
            .post(view_path, js!(&queries))?
            .send()
            .await?
            .error_for_status()?;
        let results: QueriesCollection = response.json().await?;
        Ok(results.results)
    }

    /// Gets all the documents in database, with applied parameters.
    /// Parameters description can be found here: [api-ddoc-view](https://docs.couchdb.org/en/latest/api/ddoc/views.html#api-ddoc-view)
    pub async fn get_all_params(&self, params: Option<QueryParams>) -> Result<DocumentCollection, CouchError> {
        let mut options;
        if let Some(opts) = params {
            options = opts;
        } else {
            options = QueryParams::default();
        }

        options.include_docs = Some(true);

        // we use POST here, because this allows for a larger set of keys to be provided, compared
        // to a GET call. It provides the same functionality
        let response = self
            ._client
            .post(self.create_document_path("_all_docs"), js!(&options))?
            .send()
            .await?
            .error_for_status()?;

        Ok(DocumentCollection::new(response.json().await?))
    }

    /// Finds a document in the database through a Mango query.
    /// Usage:
    /// ```
    /// use couch_rs::types::find::FindQuery;
    /// use std::error::Error;
    ///
    /// const DB_HOST: &str = "http://admin:password@localhost:5984";
    /// const TEST_DB: &str = "test_db";
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let client = couch_rs::Client::new(DB_HOST)?;
    ///     let db = client.db(TEST_DB).await?;
    ///     let find_all = FindQuery::find_all();
    ///     let docs = db.find(&find_all).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn find(&self, query: &FindQuery) -> Result<DocumentCollection, CouchError> {
        let path = self.create_document_path("_find");
        let response = self._client.post(path, js!(query))?.send().await?;
        let status = response.status();
        let data: FindResult = response.json().await.unwrap();

        if let Some(doc_val) = data.docs {
            let documents: Vec<Document> = doc_val
                .into_iter()
                .filter(|d| {
                    // Remove _design documents
                    let id: String = json_extr!(d["_id"]);
                    !id.starts_with('_')
                })
                .map(Document::new)
                .collect();

            let mut bookmark = Option::None;
            let returned_bookmark = data.bookmark.unwrap_or_default();

            if returned_bookmark != "nil" && returned_bookmark != "" {
                // a valid bookmark has been returned
                bookmark.replace(returned_bookmark);
            }

            Ok(DocumentCollection::new_from_documents(documents, bookmark))
        } else if let Some(err) = data.error {
            Err(CouchError::new(err, status))
        } else {
            Ok(DocumentCollection::default())
        }
    }

    /// Saves a document to CouchDB. When the provided document includes both an `_id` and a `_rev`
    /// CouchDB will attempt to update the document. When only an `_id` is provided, the `save`
    /// method behaves like `create` and will attempt to create the document.
    ///
    /// Usage:
    /// ```
    /// use couch_rs::types::find::FindQuery;
    /// use std::error::Error;
    /// use serde_json::{from_value, to_value};
    /// use couch_rs::types::document::DocumentId;
    ///
    /// const DB_HOST: &str = "http://admin:password@localhost:5984";
    /// const TEST_DB: &str = "test_db";
    ///
    /// #[derive(serde::Serialize, serde::Deserialize)]
    /// pub struct UserDetails {
    ///     pub _id: DocumentId,
    ///     #[serde(skip_serializing)]
    ///     pub _rev: String,
    ///     #[serde(rename = "firstName")]
    ///     pub first_name: Option<String>,
    ///     #[serde(rename = "lastName")]
    ///     pub last_name: String,
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let client = couch_rs::Client::new(DB_HOST)?;
    ///     let db = client.db(TEST_DB).await?;
    ///
    ///     // before we can get the document, we need to create it first...
    ///     let seed_doc = UserDetails {
    ///         _id: "123".to_string(),
    ///         _rev: "".to_string(),
    ///         first_name: None,
    ///         last_name: "Doe".to_string(),
    ///     };
    ///     let value = to_value(seed_doc)?;
    ///     db.create(value).await?;
    ///
    ///     // now that the document is created, we can get it, update it, and save it...
    ///     let mut doc = db.get("123").await?;
    ///     let mut user_details: UserDetails = from_value(doc.get_data())?;
    ///     user_details.first_name = Some("John".to_string());
    ///     let value = to_value(user_details)?;
    ///     doc.merge(value);
    ///
    ///     db.save(doc).await?;
    ///     Ok(())
    /// }
    ///```
    pub async fn save(&self, doc: Document) -> Result<Document, CouchError> {
        let id = doc._id.to_owned();
        let raw = doc.get_data();

        let response = self
            ._client
            .put(self.create_document_path(&id), to_string(&raw)?)?
            .send()
            .await?;

        let status = response.status();
        let data: DocumentCreatedResult = response.json().await?;

        match data.ok {
            Some(true) => {
                let mut val = doc.get_data();
                val["_rev"] = json!(data.rev);

                Ok(Document::new(val))
            }
            _ => {
                let err = data.error.unwrap_or_else(|| s!("unspecified error"));
                Err(CouchError::new(err, status))
            }
        }
    }

    /// Creates a document from a raw JSON document Value.
    pub async fn create(&self, raw_doc: Value) -> Result<Document, CouchError> {
        let response = self
            ._client
            .post(self.name.clone(), to_string(&raw_doc)?)?
            .send()
            .await?;

        let status = response.status();
        let data: DocumentCreatedResult = response.json().await?;

        match data.ok {
            Some(true) => {
                let data_id = match data.id {
                    Some(id) => id,
                    _ => return Err(CouchError::new(s!("invalid id"), status)),
                };

                let data_rev = match data.rev {
                    Some(rev) => rev,
                    _ => return Err(CouchError::new(s!("invalid rev"), status)),
                };

                let mut val = raw_doc.clone();
                val["_id"] = json!(data_id);
                val["_rev"] = json!(data_rev);

                Ok(Document::new(val))
            }
            _ => {
                let err = data.error.unwrap_or_else(|| s!("unspecified error"));
                Err(CouchError::new(err, status))
            }
        }
    }

    /// The upsert function combines a `get` with a `save` function. If the document with the
    /// provided `_id` can be found it will be merged with the provided Document's value, otherwise
    /// the document will be created.
    /// This operation always performs a `get`, so if you have a documents `_rev` using a `save` is
    /// quicker. Same is true when you know a document does *not* exist.
    ///
    /// Usage:
    ///
    /// ```
    /// use couch_rs::types::find::FindQuery;
    /// use couch_rs::document::Document;
    /// use std::error::Error;
    /// use serde_json::json;
    ///
    /// const DB_HOST: &str = "http://admin:password@localhost:5984";
    /// const TEST_DB: &str = "test_db";
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let client = couch_rs::Client::new(DB_HOST)?;
    ///     let db = client.db(TEST_DB).await?;
    ///     let doc = Document::new(json!({
    ///                     "_id": "doe",
    ///                     "first_name": "John",
    ///                     "last_name": "Doe"
    ///                 }));
    ///
    ///     // initial call creates the document
    ///     db.upsert(doc.clone()).await?;
    ///
    ///     // subsequent call updates the existing document
    ///     let updated_doc = db.upsert(doc).await?;
    ///
    ///     // verify that this is the 2nd revision of the document
    ///     assert!(updated_doc._rev.starts_with('2'));
    ///     Ok(())
    /// }
    /// ```
    pub async fn upsert(&self, doc: Document) -> Result<Document, CouchError> {
        let id = doc._id.clone();

        match self.get(&id).await {
            Ok(mut current_doc) => {
                current_doc.merge(doc.get_data());
                let doc = self.save(current_doc).await?;
                Ok(doc)
            }
            Err(err) => {
                if err.is_not_found() {
                    // document does not yet exist
                    let doc = self.save(doc).await?;
                    Ok(doc)
                } else {
                    Err(err)
                }
            }
        }
    }

    /// Creates a design with one of more view documents.
    ///
    /// Usage:
    /// ```
    /// use couch_rs::types::view::{CouchFunc, CouchViews};
    /// use std::error::Error;
    ///
    /// const DB_HOST: &str = "http://admin:password@localhost:5984";
    /// const TEST_DB: &str = "test_db";
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let client = couch_rs::Client::new(DB_HOST)?;
    ///     let db = client.db(TEST_DB).await?;
    ///
    ///     let couch_func = CouchFunc {
    ///             map: "function (doc) { if (doc.CLIP == true) { emit(doc.CLIP); } }".to_string(),
    ///             reduce: None,
    ///         };
    ///
    ///     let couch_views = CouchViews::new("clip_view", couch_func);
    ///     db.create_view("clip_design".to_string(), couch_views).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn create_view<T: Into<serde_json::Value>>(
        &self,
        design_name: String,
        views: T,
    ) -> Result<DesignCreated, CouchError> {
        let doc: Value = views.into();
        let response = self
            ._client
            .put(self.create_design_path(&design_name), to_string(&doc)?)?
            .send()
            .await?;

        let response_status = response.status();
        let result: DesignCreated = response.json().await?;

        if response_status.is_success() {
            Ok(result)
        } else {
            match result.error {
                Some(e) => Err(CouchError {
                    status: response_status,
                    message: e,
                }),
                None => Err(CouchError {
                    status: response_status,
                    message: s!("unspecified error"),
                }),
            }
        }
    }

    /// Executes a query against a view.
    pub async fn query(
        &self,
        design_name: &str,
        view_name: &str,
        mut options: Option<QueryParams>,
    ) -> Result<ViewCollection, CouchError> {
        if options.is_none() {
            options = Some(QueryParams::default());
        }

        let response = self
            ._client
            .post(self.create_query_view_path(design_name, view_name), js!(&options))?
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }

    /// Executes an update function.
    pub async fn execute_update(
        &self,
        design_id: &str,
        name: &str,
        document_id: &str,
        body: Option<Value>,
    ) -> Result<String, CouchError> {
        let body = match body {
            Some(v) => to_string(&v)?,
            None => "".to_string(),
        };

        let response = self
            ._client
            .put(self.create_execute_update_path(design_id, name, document_id), body)?
            .send()
            .await?
            .error_for_status()?;

        Ok(response.text().await?)
    }

    /// Removes a document from the database. Returns success in a `bool`
    /// Usage:
    /// ```
    /// use couch_rs::types::find::FindQuery;
    /// use std::error::Error;
    /// use serde_json::{from_value, to_value};
    /// use couch_rs::types::document::DocumentId;
    /// use couch_rs::document::Document;
    ///
    /// const DB_HOST: &str = "http://admin:password@localhost:5984";
    /// const TEST_DB: &str = "test_db";
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let client = couch_rs::Client::new(DB_HOST)?;
    ///     let db = client.db(TEST_DB).await?;
    ///
    ///     // first we need to get the document, because we need both the _id and _rev in order
    ///     // to delete
    ///     if let Some(doc) = db.get("123").await.ok() {
    ///         db.remove(doc).await;
    ///     }
    ///
    ///     Ok(())
    /// }
    ///```     
    pub async fn remove(&self, doc: Document) -> bool {
        let request = self._client.delete(
            self.create_document_path(&doc._id),
            Some({
                let mut h = HashMap::new();
                h.insert(s!("rev"), doc._rev.clone());
                h
            }),
        );

        self.is_ok(request).await
    }

    /// Inserts an index in a naive way, if it already exists, will throw an
    /// `Err`
    pub async fn insert_index(&self, name: String, spec: IndexFields) -> Result<DesignCreated, CouchError> {
        let response = self
            ._client
            .post(
                self.create_document_path("_index"),
                js!(json!({
                    "name": name,
                    "index": spec
                })),
            )?
            .send()
            .await?;

        let status = response.status();
        let data: DesignCreated = response.json().await?;

        if data.error.is_some() {
            let err = data.error.unwrap_or_else(|| s!("unspecified error"));
            Err(CouchError::new(err, status))
        } else {
            Ok(data)
        }
    }

    /// Reads the database's indexes and returns them
    pub async fn read_indexes(&self) -> Result<DatabaseIndexList, CouchError> {
        let response = self
            ._client
            .get(self.create_document_path("_index"), None)?
            .send()
            .await?;

        Ok(response.json().await?)
    }

    /// Method to ensure an index is created on the database with the following
    /// spec. Returns `true` when we created a new one, or `false` when the
    /// index was already existing.
    pub async fn ensure_index(&self, name: String, spec: IndexFields) -> Result<bool, CouchError> {
        let db_indexes = self.read_indexes().await?;

        // We look for our index
        for i in db_indexes.indexes.into_iter() {
            if i.name == name {
                // Found? Ok let's return
                return Ok(false);
            }
        }

        // Let's create it then
        let result: DesignCreated = self.insert_index(name, spec).await?;
        match result.error {
            Some(e) => Err(CouchError {
                status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                message: e,
            }),
            // Created and alright
            None => Ok(true),
        }
    }
}
