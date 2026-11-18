
use crate::ingest::{self, Rejection};
use futures_util::TryStreamExt;
use miaudit_schema::{
    digest::sha256_hex,
    media::image_mime,
    model::{EvidenceBody, Manifest, Stored, StoredBinding},
};
use miaudit_ui::data::{EvidenceView, Item, View};
use mongodb::{
    Client, Collection, Database, IndexModel,
    bson::{Binary, Document, doc, spec::BinarySubtype},
    options::IndexOptions,
};
use serde::{Deserialize, Serialize};

pub const PAGE_SIZE: i64 = 25;

#[derive(Debug)]
pub enum DbError {
    Mongo(mongodb::error::Error),
    Rejected(Rejection),
    Conflict(String),
    NotFound(String),
}

impl From<mongodb::error::Error> for DbError {
    fn from(e: mongodb::error::Error) -> Self {
        DbError::Mongo(e)
    }
}
impl From<Rejection> for DbError {
    fn from(r: Rejection) -> Self {
        DbError::Rejected(r)
    }
}

type R<T> = Result<T, DbError>;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RepoDoc {
    #[serde(rename = "_id")]
    pub repo_id: String,
    pub slug: String,
    pub remote_url: Option<String>,
    pub created_at: String,
    pub last_push_at: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AuditDoc {
    pub repo_id: String,
    pub audit_id: String,
    pub raw: String,
    pub created_at: String,
    pub description: String,
    pub author_login: Option<String>,
    pub changes: i64,
    pub gaps: i64,
    #[serde(default)]
    pub gap_codes: Vec<String>,
    pub parents: Vec<String>,
    pub received_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ObjectDoc {
    repo_id: String,
    id: String,
    raw: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct BlobDoc {
    repo_id: String,
    digest: String,
    size: i64,
    image: bool,
    preview: Option<String>,
    data: Binary,
}

#[derive(Deserialize, Debug)]
struct BlobMeta {
    digest: String,
    image: bool,
    preview: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct BindingDoc {
    repo_id: String,
    binding_id: String,
    audit_id: String,
    raw: String,
    created_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TokenDoc {
    pub name: String,
    pub hash: String,
    pub created_at: String,
    pub revoked: bool,
}

#[derive(Debug, Clone)]
pub struct RepoStats {
    pub repo: RepoDoc,
    pub audits: i64,
    pub gaps: i64,
    pub last_audit_at: Option<String>,
}

#[derive(Clone)]
pub struct Db {
    name: String,
    repos: Collection<RepoDoc>,
    audits: Collection<AuditDoc>,
    objects: Collection<ObjectDoc>,
    blobs: Collection<BlobDoc>,
    bindings: Collection<BindingDoc>,
    tokens: Collection<TokenDoc>,
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn unique() -> IndexOptions {
    IndexOptions::builder().unique(true).build()
}

fn idx(keys: Document, unique_opt: bool) -> IndexModel {
    let b = IndexModel::builder().keys(keys);
    if unique_opt { b.options(unique()).build() } else { b.build() }
}

pub fn hash_token(token: &str) -> String {
    sha256_hex(token.as_bytes())
}

impl Db {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn connect(uri: &str, name: &str) -> R<Db> {
        let client = Client::with_uri_str(uri).await?;
        Self::open(client.database(name)).await
    }

    pub async fn open(db: Database) -> R<Db> {
        let s = Db {
            name: db.name().to_string(),
            repos: db.collection("repos"),
            audits: db.collection("audits"),
            objects: db.collection("objects"),
            blobs: db.collection("blobs"),
            bindings: db.collection("bindings"),
            tokens: db.collection("tokens"),
        };
        s.repos.create_indexes([idx(doc! {"slug": 1}, true)]).await?;
        s.audits
            .create_indexes([
                idx(doc! {"repo_id": 1, "audit_id": 1}, true),
                idx(doc! {"repo_id": 1, "created_at": -1, "audit_id": 1}, false),
                idx(doc! {"repo_id": 1, "author_login": 1}, false),
            ])
            .await?;
        s.objects.create_indexes([idx(doc! {"repo_id": 1, "id": 1}, true)]).await?;
        s.blobs.create_indexes([idx(doc! {"repo_id": 1, "digest": 1}, true)]).await?;
        s.bindings.create_indexes([idx(doc! {"repo_id": 1, "binding_id": 1}, true), idx(doc! {"repo_id": 1, "audit_id": 1}, false)]).await?;
        s.tokens.create_indexes([idx(doc! {"hash": 1}, true), idx(doc! {"name": 1}, true)]).await?;
        s.backfill_gap_codes().await?;
        Ok(s)
    }

    async fn backfill_gap_codes(&self) -> R<()> {
        let old: Vec<AuditDoc> = self.audits.find(doc! {"gap_codes": {"$exists": false}}).await?.try_collect().await?;
        for a in old {
            if let Ok(m) = serde_json::from_str::<Manifest>(&a.raw) {
                let codes: Vec<String> = m.gaps.iter().map(|g| g.code.clone()).collect();
                self.audits.update_one(doc! {"repo_id": &a.repo_id, "audit_id": &a.audit_id}, doc! {"$set": {"gap_codes": codes}}).await?;
            }
        }
        Ok(())
    }


    pub async fn create_token(&self, name: &str, token: &str) -> R<()> {
        let d = TokenDoc { name: name.into(), hash: hash_token(token), created_at: now(), revoked: false };
        match self.tokens.insert_one(d).await {
            Ok(_) => Ok(()),
            Err(e) if is_dup(&e) => Err(DbError::Conflict(format!("a token named {name:?} already exists"))),
            Err(e) => Err(e.into()),
        }
    }
    pub async fn token_valid(&self, token: &str) -> R<bool> {
        Ok(self.tokens.find_one(doc! {"hash": hash_token(token), "revoked": false}).await?.is_some())
    }
    pub async fn list_tokens(&self) -> R<Vec<TokenDoc>> {
        Ok(self.tokens.find(doc! {}).sort(doc! {"created_at": 1}).await?.try_collect().await?)
    }
    pub async fn revoke_token(&self, name: &str) -> R<bool> {
        Ok(self.tokens.update_one(doc! {"name": name, "revoked": false}, doc! {"$set": {"revoked": true}}).await?.modified_count > 0)
    }


    pub async fn register_repo(&self, repo_id: &str, slug: &str, remote_url: Option<&str>) -> R<RepoDoc> {
        if !ingest::valid_repo_id(repo_id) {
            return Err(Rejection { code: "BAD_ID", message: "repo id must be repo_ followed by 32 hex".into() }.into());
        }
        if !ingest::valid_slug(slug) {
            return Err(Rejection { code: "BAD_SLUG", message: "slug must be owner/name using letters, digits, '.', '_' or '-'".into() }.into());
        }
        if let Some(other) = self.repos.find_one(doc! {"slug": slug, "_id": {"$ne": repo_id}}).await? {
            return Err(DbError::Conflict(format!("slug {slug} already belongs to {}; push with a different --name", other.repo_id)));
        }
        let update = doc! {
            "$set": {"slug": slug, "remote_url": remote_url},
            "$setOnInsert": {"created_at": now()},
        };
        match self.repos.update_one(doc! {"_id": repo_id}, update).upsert(true).await {
            Ok(_) => {}
            Err(e) if is_dup(&e) => return Err(DbError::Conflict(format!("slug {slug} is already taken"))),
            Err(e) => return Err(e.into()),
        }
        self.repo_by_id(repo_id).await?.ok_or_else(|| DbError::NotFound("repo vanished".into()))
    }

    pub async fn repo_by_id(&self, repo_id: &str) -> R<Option<RepoDoc>> {
        Ok(self.repos.find_one(doc! {"_id": repo_id}).await?)
    }
    pub async fn repo_by_slug(&self, slug: &str) -> R<Option<RepoDoc>> {
        Ok(self.repos.find_one(doc! {"slug": slug}).await?)
    }

    async fn touch(&self, repo_id: &str) -> R<()> {
        self.repos.update_one(doc! {"_id": repo_id}, doc! {"$set": {"last_push_at": now()}}).await?;
        Ok(())
    }

    pub async fn repo_stats(&self, q: Option<&str>) -> R<Vec<RepoStats>> {
        let filter = match q.map(str::trim).filter(|q| !q.is_empty()) {
            Some(q) => doc! {"slug": {"$regex": regex_escape(q), "$options": "i"}},
            None => doc! {},
        };
        let repos: Vec<RepoDoc> = self.repos.find(filter).await?.try_collect().await?;
        let mut cursor = self
            .audits
            .clone_with_type::<Document>()
            .aggregate([doc! {"$group": {"_id": "$repo_id", "n": {"$sum": 1}, "gaps": {"$sum": "$gaps"}, "last": {"$max": "$created_at"}}}])
            .await?;
        let mut agg = std::collections::HashMap::new();
        while let Some(d) = cursor.try_next().await? {
            let id = d.get_str("_id").unwrap_or_default().to_string();
            let n = d.get_i32("n").map(i64::from).or_else(|_| d.get_i64("n")).unwrap_or(0);
            let gaps = d.get_i32("gaps").map(i64::from).or_else(|_| d.get_i64("gaps")).unwrap_or(0);
            agg.insert(id, (n, gaps, d.get_str("last").ok().map(String::from)));
        }
        let mut out: Vec<RepoStats> = repos
            .into_iter()
            .map(|repo| {
                let (audits, gaps, last_audit_at) = agg.remove(&repo.repo_id).unwrap_or((0, 0, None));
                RepoStats { repo, audits, gaps, last_audit_at }
            })
            .collect();
        out.sort_by(|a, b| b.repo.last_push_at.cmp(&a.repo.last_push_at).then(a.repo.slug.cmp(&b.repo.slug)));
        Ok(out)
    }


    pub async fn put_blob(&self, repo_id: &str, digest: &str, bytes: &[u8]) -> R<bool> {
        ingest::check_blob(digest, bytes)?;
        self.require_repo(repo_id).await?;
        if self.blobs.count_documents(doc! {"repo_id": repo_id, "digest": digest}).limit(1).await? > 0 {
            return Ok(false);
        }
        let image = image_mime(bytes).is_some();
        let preview = (!image && bytes.len() <= 8192).then(|| String::from_utf8(bytes.to_vec()).ok()).flatten();
        let d = BlobDoc {
            repo_id: repo_id.into(),
            digest: digest.into(),
            size: bytes.len() as i64,
            image,
            preview,
            data: Binary { subtype: BinarySubtype::Generic, bytes: bytes.to_vec() },
        };
        self.insert_new(&self.blobs, d).await?;
        self.touch(repo_id).await?;
        Ok(true)
    }

    pub async fn put_object(&self, repo_id: &str, id: &str, bytes: &[u8]) -> R<bool> {
        ingest::check_object(id, bytes)?;
        self.require_repo(repo_id).await?;
        if self.objects.count_documents(doc! {"repo_id": repo_id, "id": id}).limit(1).await? > 0 {
            return Ok(false);
        }
        let raw = String::from_utf8(bytes.to_vec()).map_err(|_| Rejection { code: "BAD_JSON", message: "not UTF-8".into() })?;
        self.insert_new(&self.objects, ObjectDoc { repo_id: repo_id.into(), id: id.into(), raw }).await?;
        self.touch(repo_id).await?;
        Ok(true)
    }

    pub async fn put_commit(&self, repo_id: &str, id: &str, bytes: &[u8]) -> R<bool> {
        let m = ingest::check_commit(repo_id, id, bytes)?;
        self.require_repo(repo_id).await?;
        if self.audits.count_documents(doc! {"repo_id": repo_id, "audit_id": id}).limit(1).await? > 0 {
            return Ok(false);
        }
        let raw = String::from_utf8(bytes.to_vec()).map_err(|_| Rejection { code: "BAD_JSON", message: "not UTF-8".into() })?;
        let d = AuditDoc {
            repo_id: repo_id.into(),
            audit_id: id.into(),
            raw,
            created_at: m.created.at.clone(),
            description: m.description.clone(),
            author_login: m.author.as_ref().and_then(|a| a.github_login.clone()),
            changes: m.snapshot.changes.len() as i64,
            gaps: m.gaps.len() as i64,
            gap_codes: m.gaps.iter().map(|g| g.code.clone()).collect(),
            parents: m.parents.clone(),
            received_at: now(),
        };
        self.insert_new(&self.audits, d).await?;
        self.touch(repo_id).await?;
        Ok(true)
    }

    pub async fn put_binding(&self, repo_id: &str, id: &str, bytes: &[u8]) -> R<bool> {
        let b = ingest::check_binding(id, bytes)?;
        self.require_repo(repo_id).await?;
        if self.audits.count_documents(doc! {"repo_id": repo_id, "audit_id": &b.body.audit_id}).limit(1).await? == 0 {
            return Err(DbError::Conflict(format!("audit commit {} has not been pushed to this repo yet", b.body.audit_id)));
        }
        if self.bindings.count_documents(doc! {"repo_id": repo_id, "binding_id": id}).limit(1).await? > 0 {
            return Ok(false);
        }
        let raw = String::from_utf8(bytes.to_vec()).map_err(|_| Rejection { code: "BAD_JSON", message: "not UTF-8".into() })?;
        let d = BindingDoc { repo_id: repo_id.into(), binding_id: id.into(), audit_id: b.body.audit_id.clone(), raw, created_at: b.created.at.clone() };
        self.insert_new(&self.bindings, d).await?;
        self.touch(repo_id).await?;
        Ok(true)
    }

    async fn insert_new<T: Serialize + Send + Sync>(&self, c: &Collection<T>, d: T) -> R<()> {
        match c.insert_one(d).await {
            Ok(_) => Ok(()),
            Err(e) if is_dup(&e) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn require_repo(&self, repo_id: &str) -> R<()> {
        if self.repos.count_documents(doc! {"_id": repo_id}).await? == 0 {
            return Err(DbError::NotFound(format!("repo {repo_id} is not registered; register it first")));
        }
        Ok(())
    }

    pub async fn missing(&self, repo_id: &str, commits: &[String], objects: &[String], blobs: &[String], bindings: &[String]) -> R<[Vec<String>; 4]> {
        async fn absent<T: Send + Sync + serde::de::DeserializeOwned + Unpin>(c: &Collection<T>, repo_id: &str, field: &str, ids: &[String]) -> R<Vec<String>> {
            if ids.is_empty() {
                return Ok(vec![]);
            }
            let have: Vec<Document> = c
                .clone_with_type::<Document>()
                .find(doc! {"repo_id": repo_id, field: {"$in": ids}})
                .projection(doc! {field: 1, "_id": 0})
                .await?
                .try_collect()
                .await?;
            let have: std::collections::HashSet<&str> = have.iter().filter_map(|d| d.get_str(field).ok()).collect();
            Ok(ids.iter().filter(|i| !have.contains(i.as_str())).cloned().collect())
        }
        Ok([
            absent(&self.audits, repo_id, "audit_id", commits).await?,
            absent(&self.objects, repo_id, "id", objects).await?,
            absent(&self.blobs, repo_id, "digest", blobs).await?,
            absent(&self.bindings, repo_id, "binding_id", bindings).await?,
        ])
    }


    pub async fn audit_page(&self, repo_id: &str, author: Option<&str>, gap: Option<&str>, page: u64) -> R<(Vec<AuditDoc>, i64)> {
        let mut filter = doc! {"repo_id": repo_id};
        if let Some(a) = author.filter(|a| !a.is_empty()) {
            filter.insert("author_login", a);
        }
        if let Some(g) = gap.filter(|g| !g.is_empty()) {
            filter.insert("gap_codes", doc! {"$regex": format!("^{}", regex_escape(g))});
        }
        let total = self.audits.count_documents(filter.clone()).await? as i64;
        let rows = self
            .audits
            .find(filter)
            .sort(doc! {"created_at": -1, "audit_id": 1})
            .skip(page.saturating_sub(1) * PAGE_SIZE as u64)
            .limit(PAGE_SIZE)
            .await?
            .try_collect()
            .await?;
        Ok((rows, total))
    }

    pub async fn authors(&self, repo_id: &str) -> R<Vec<(String, i64)>> {
        let mut c = self
            .audits
            .clone_with_type::<Document>()
            .aggregate([
                doc! {"$match": {"repo_id": repo_id, "author_login": {"$ne": null}}},
                doc! {"$group": {"_id": "$author_login", "n": {"$sum": 1}}},
                doc! {"$sort": {"n": -1, "_id": 1}},
                doc! {"$limit": 20},
            ])
            .await?;
        let mut out = vec![];
        while let Some(d) = c.try_next().await? {
            let n = d.get_i32("n").map(i64::from).or_else(|_| d.get_i64("n")).unwrap_or(0);
            out.push((d.get_str("_id").unwrap_or_default().to_string(), n));
        }
        Ok(out)
    }

    pub async fn bindings_for(&self, repo_id: &str, audit_ids: &[String]) -> R<Vec<StoredBinding>> {
        if audit_ids.is_empty() {
            return Ok(vec![]);
        }
        let docs: Vec<BindingDoc> = self.bindings.find(doc! {"repo_id": repo_id, "audit_id": {"$in": audit_ids}}).sort(doc! {"created_at": 1}).await?.try_collect().await?;
        Ok(docs.into_iter().filter_map(|d| serde_json::from_str(&d.raw).ok()).collect())
    }

    pub async fn raw_commit(&self, repo_id: &str, audit_id: &str) -> R<Option<String>> {
        Ok(self.audits.find_one(doc! {"repo_id": repo_id, "audit_id": audit_id}).await?.map(|d| d.raw))
    }

    pub async fn commit_exists(&self, repo_id: &str, audit_id: &str) -> R<bool> {
        Ok(self.audits.count_documents(doc! {"repo_id": repo_id, "audit_id": audit_id}).await? > 0)
    }

    async fn objects_raw(&self, repo_id: &str, ids: &[String]) -> R<std::collections::HashMap<String, String>> {
        if ids.is_empty() {
            return Ok(Default::default());
        }
        let docs: Vec<ObjectDoc> = self.objects.find(doc! {"repo_id": repo_id, "id": {"$in": ids}}).await?.try_collect().await?;
        Ok(docs.into_iter().map(|d| (d.id, d.raw)).collect())
    }

    async fn blob_meta(&self, repo_id: &str, digests: &[String]) -> R<std::collections::HashMap<String, (bool, Option<String>)>> {
        if digests.is_empty() {
            return Ok(Default::default());
        }
        let docs: Vec<BlobMeta> = self
            .blobs
            .clone_with_type::<BlobMeta>()
            .find(doc! {"repo_id": repo_id, "digest": {"$in": digests}})
            .projection(doc! {"data": 0})
            .await?
            .try_collect()
            .await?;
        Ok(docs.into_iter().map(|d| (d.digest, (d.image, d.preview))).collect())
    }

    pub async fn blob_bytes(&self, repo_id: &str, digest: &str) -> R<Option<Vec<u8>>> {
        Ok(self.blobs.find_one(doc! {"repo_id": repo_id, "digest": digest}).await?.map(|d| d.data.bytes))
    }

    pub async fn evidence_image(&self, repo_id: &str, audit_id: &str, evidence_id: &str) -> R<Option<(&'static str, Vec<u8>)>> {
        let Some(raw) = self.raw_commit(repo_id, audit_id).await? else { return Ok(None) };
        let Ok(m) = serde_json::from_str::<Manifest>(&raw) else { return Ok(None) };
        if !m.evidence_ids.iter().any(|e| e == evidence_id) {
            return Ok(None);
        }
        let objs = self.objects_raw(repo_id, &[evidence_id.to_string()]).await?;
        let Some(ev) = objs.get(evidence_id).and_then(|r| serde_json::from_str::<Stored<EvidenceBody>>(r).ok()) else { return Ok(None) };
        let digest = ev.body.artifact.digest;
        let Some(bytes) = self.blob_bytes(repo_id, &digest).await? else { return Ok(None) };
        if bytes.len() > 10 * 1024 * 1024 || miaudit_schema::digest::content_id(&bytes) != digest {
            return Ok(None);
        }
        Ok(image_mime(&bytes).map(|m| (m, bytes)))
    }

    pub async fn load_view(&self, repo_id: &str, audit_id: &str) -> R<Option<(View, Vec<(String, String, String)>)>> {
        let Some(raw) = self.raw_commit(repo_id, audit_id).await? else { return Ok(None) };
        let Ok(m) = serde_json::from_str::<Manifest>(&raw) else { return Ok(None) };
        let bindings = self.bindings_for(repo_id, &[audit_id.to_string()]).await?;
        let mut all: Vec<String> = m.context_ids.iter().chain(&m.decision_ids).chain(&m.evidence_ids).cloned().collect();
        all.sort();
        all.dedup();
        let raws = self.objects_raw(repo_id, &all).await?;

        fn parse<T: serde::de::DeserializeOwned>(raws: &std::collections::HashMap<String, String>, id: &str) -> Option<T> {
            raws.get(id).and_then(|r| serde_json::from_str(r).ok())
        }
        let ev_objs: Vec<(String, Option<Stored<EvidenceBody>>)> = m.evidence_ids.iter().map(|id| (id.clone(), parse(&raws, id))).collect();
        let digests: Vec<String> = ev_objs.iter().filter_map(|(_, o)| o.as_ref().map(|o| o.body.artifact.digest.clone())).collect();
        let meta = self.blob_meta(repo_id, &digests).await?;
        let evidence: Vec<EvidenceView> = ev_objs
            .into_iter()
            .map(|(id, object)| {
                let (mut preview, mut state, mut image) = (None, "object unavailable", false);
                if let Some(o) = &object {
                    match meta.get(&o.body.artifact.digest) {
                        None => state = "artifact unavailable",
                        Some((is_img, prev)) => {
                            image = *is_img;
                            state = "present (digest checked when stored)";
                            preview = prev.clone();
                            if !image && preview.is_none() {
                                state = "present, too large to preview";
                            }
                        }
                    }
                }
                EvidenceView { id, object, preview, artifact_state: state, image }
            })
            .collect();

        let mut checks: Vec<(String, String, String)> = vec![];
        let mut add = |n: &str, ok: Option<bool>, d: String| checks.push((n.into(), match ok { Some(true) => "pass", Some(false) => "fail", None => "partial" }.into(), d));
        add("manifest_parses", Some(true), "closed schema".into());
        match m.audit_id() {
            Ok(got) if got == audit_id => add("audit_id_matches_canonical_manifest", Some(true), got),
            Ok(got) => add("audit_id_matches_canonical_manifest", Some(false), format!("recomputed {got}")),
            Err(e) => add("audit_id_matches_canonical_manifest", Some(false), e.to_string()),
        }
        let mut missing_parents = vec![];
        for p in &m.parents {
            if !self.commit_exists(repo_id, p).await? {
                missing_parents.push(p.clone());
            }
        }
        if missing_parents.is_empty() {
            add("parents_available", Some(true), format!("{} parent(s)", m.parents.len()));
        } else {
            add("parents_available", None, format!("not on the server: {missing_parents:?}"));
        }
        for (name, ids) in [("contexts", &m.context_ids), ("decisions", &m.decision_ids), ("evidence", &m.evidence_ids)] {
            let missing: Vec<&String> = ids.iter().filter(|i| !raws.contains_key(*i)).collect();
            if missing.is_empty() {
                add(&format!("{name}_integrity"), Some(true), format!("{} object(s), hashed when stored", ids.len()));
            } else {
                add(&format!("{name}_integrity"), None, format!("not on the server: {missing:?}"));
            }
        }
        let unavailable = evidence.iter().filter(|e| e.object.is_some() && e.artifact_state == "artifact unavailable").count();
        if unavailable == 0 {
            add("evidence_artifacts", Some(true), format!("{} artifact(s) present", evidence.iter().filter(|e| e.object.is_some()).count()));
        } else {
            add("evidence_artifacts", None, format!("{unavailable} artifact(s) not on the server"));
        }

        let view = View {
            id: audit_id.into(),
            manifest: m.clone(),
            bindings,
            contexts: m.context_ids.iter().map(|i| Item { id: i.clone(), object: parse(&raws, i) }).collect(),
            decisions: m.decision_ids.iter().map(|i| Item { id: i.clone(), object: parse(&raws, i) }).collect(),
            evidence,
        };
        Ok(Some((view, checks)))
    }
}

fn is_dup(e: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    matches!(e.kind.as_ref(), ErrorKind::Write(WriteFailure::WriteError(w)) if w.code == 11000)
}

fn regex_escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            o.push('\\');
        }
        o.push(c);
    }
    o
}
