//! Version 2 experiment: one durable file, historical read keys and admission-aware replay.
//! This is a single-writer scenario model, not the production Taypeer protocol.
use crate::*;
use automerge::{
    ActorId, Automerge, Change, ChangeHash, ObjId, ObjType, ROOT, ReadDoc, ScalarValue, Value,
    transaction::{CommitOptions, Transactable, Transaction},
};
use std::{cell::Cell, collections::BTreeMap, path::PathBuf};

#[derive(Clone, Serialize, Deserialize)]
struct Ciphertext {
    nonce: [u8; 24],
    bytes: Vec<u8>,
}
impl Ciphertext {
    fn seal(key: &Id, aad: &[u8], clear: &[u8]) -> Result<Self> {
        let nonce = random();
        let bytes = XChaCha20Poly1305::new(key.into())
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: clear, aad })
            .map_err(|_| "encrypt")?;
        Ok(Self { nonce, bytes })
    }
    fn open(&self, key: &Id, aad: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        XChaCha20Poly1305::new(key.into())
            .decrypt(
                XNonce::from_slice(&self.nonce),
                Payload {
                    msg: &self.bytes,
                    aad,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| "authentication")
    }
}
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| "encode")
}
fn verify(author: Id, bytes: &[u8], signature: &[u8]) -> Result<()> {
    VerifyingKey::from_bytes(&author)
        .map_err(|_| "author")?
        .verify_strict(
            bytes,
            &Signature::from_slice(signature).map_err(|_| "signature")?,
        )
        .map_err(|_| "signature")
}

/// No private admission or management credentials are serialized here.
#[derive(Clone, Serialize, Deserialize)]
pub struct State {
    pub group: [u8; 16],
    pub epoch: u64,
    pub control: Id,
    pub document: Vec<u8>,
    /// Unbounded retention by policy; the experiment's total size cap still applies.
    pub keys: BTreeMap<u64, Id>,
    pub applied: BTreeSet<Id>,
    #[serde(default)]
    pub kdf: crate::kdf::Profile,
}
impl State {
    pub fn empty(control: &Control) -> Self {
        Self {
            group: control.group,
            epoch: control.epoch,
            control: control.hash(),
            document: Automerge::new().save(),
            keys: [(control.epoch, random())].into(),
            applied: BTreeSet::new(),
            kdf: crate::kdf::Profile::default(),
        }
    }
    pub fn doc(&self) -> Result<Automerge> {
        let doc = Automerge::load(&self.document).map_err(|_| "document")?;
        if !doc.get_missing_deps(&[]).is_empty() {
            return Err("incomplete snapshot");
        }
        Ok(doc)
    }
}

/// The password wraps ONLY the current independent content key. The key encrypts
/// the document and historical keyring. Old content can be opened without its KEK.
#[derive(Clone, Serialize, Deserialize)]
pub struct SealedState {
    version: u16,
    group: [u8; 16],
    epoch: u64,
    control: Id,
    memory: u32,
    iterations: u32,
    salt: [u8; 16],
    wrapped: Ciphertext,
    content: Ciphertext,
}
impl SealedState {
    fn aad(&self) -> Result<Vec<u8>> {
        encode(&(
            "Pass2P/spike/snapshot/2",
            self.version,
            self.group,
            self.epoch,
            self.control,
            self.memory,
            self.iterations,
            self.salt,
        ))
    }
    pub fn seal(state: &State, password: &[u8]) -> Result<Self> {
        let blank = Ciphertext {
            nonce: [0; 24],
            bytes: vec![],
        };
        let mut sealed = Self {
            version: 2,
            group: state.group,
            epoch: state.epoch,
            control: state.control,
            memory: state.kdf.memory,
            iterations: state.kdf.iterations,
            salt: random(),
            wrapped: blank.clone(),
            content: blank,
        };
        let kek = derive(password, &sealed.salt, sealed.memory, sealed.iterations)?;
        let key = state.keys.get(&state.epoch).ok_or("missing current key")?;
        let aad = sealed.aad()?;
        sealed.wrapped = Ciphertext::seal(&kek, &[aad.as_slice(), b"/wrap"].concat(), key)?;
        sealed.content = Ciphertext::seal(
            key,
            &[aad.as_slice(), b"/content"].concat(),
            &Zeroizing::new(encode(state)?),
        )?;
        Ok(sealed)
    }
    pub fn open(&self, password: &[u8]) -> Result<State> {
        if self.version != 2 {
            return Err("version");
        }
        let kek = derive(password, &self.salt, self.memory, self.iterations)?;
        let key = self
            .wrapped
            .open(&kek, &[self.aad()?.as_slice(), b"/wrap"].concat())?;
        self.open_key(key.as_slice().try_into().map_err(|_| "key size")?)
    }
    pub fn open_key(&self, key: &Id) -> Result<State> {
        if self.version != 2 {
            return Err("version");
        }
        let clear = self
            .content
            .open(key, &[self.aad()?.as_slice(), b"/content"].concat())?;
        let state: State = serde_json::from_slice(&clear).map_err(|_| "snapshot")?;
        if state.group != self.group
            || state.epoch != self.epoch
            || state.control != self.control
            || state.kdf.memory != self.memory
            || state.kdf.iterations != self.iterations
            || state.keys.get(&self.epoch) != Some(key)
            || state.keys.keys().any(|e| *e > self.epoch)
        {
            return Err("snapshot binding");
        }
        state.doc()?;
        Ok(state)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SignedControl {
    pub state: Control,
    pub signature: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Chain {
    pub root: Control,
    pub successors: Vec<SignedControl>,
}
impl Chain {
    pub fn head(&self) -> &Control {
        self.successors
            .last()
            .map(|c| &c.state)
            .unwrap_or(&self.root)
    }
    fn states(&self) -> Vec<&Control> {
        std::iter::once(&self.root)
            .chain(self.successors.iter().map(|c| &c.state))
            .collect()
    }
    fn validate(&self, pinned: Id) -> Result<()> {
        if self.root.hash() != pinned
            || self.root.seq != 0
            || self.root.previous != [0; 32]
            || !self.root.members.contains(&self.root.manager)
        {
            return Err("trust root");
        }
        let mut previous = &self.root;
        for next in &self.successors {
            previous.accept(&next.state, &next.signature)?;
            previous = &next.state;
        }
        Ok(())
    }
    fn origin(&self, control: Id) -> Result<usize> {
        self.states()
            .iter()
            .position(|s| s.hash() == control)
            .ok_or("unknown control")
    }
    fn continuous(&self, author: Id, control: Id) -> bool {
        self.origin(control).is_ok_and(|start| {
            self.states()[start..]
                .iter()
                .all(|s| s.members.contains(&author))
        })
    }
}

/// A second domain-separated signature binds a legacy envelope to its exact
/// admission state (epoch alone cannot distinguish revoke/re-admit sequences).
#[derive(Clone, Serialize, Deserialize)]
pub struct Packet {
    pub control: Id,
    pub envelope: Envelope,
    binding: Vec<u8>,
}
impl Packet {
    pub fn create(state: &State, author: &SigningKey, change: &Change) -> Result<Self> {
        let envelope = Envelope::create(
            state.group,
            state.epoch,
            state.keys.get(&state.epoch).ok_or("key")?,
            author,
            change.raw_bytes(),
        );
        let mut p = Self {
            control: state.control,
            envelope,
            binding: vec![],
        };
        p.binding = author.sign(&p.signed()?).to_bytes().to_vec();
        Ok(p)
    }
    fn signed(&self) -> Result<Vec<u8>> {
        encode(&("Pass2P/spike/admission/2", self.control, &self.envelope))
    }
    pub fn id(&self) -> Id {
        digest(&encode(self).expect("packet serialization"))
    }
    fn validate(&self, chain: &Chain) -> Result<()> {
        let e = &self.envelope;
        let states = chain.states();
        let origin = states[chain.origin(self.control)?];
        if e.version != 1
            || e.group != origin.group
            || e.epoch != origin.epoch
            || !origin.members.contains(&e.author)
        {
            return Err("packet admission");
        }
        verify(e.author, &e.signed(), &e.signature)?;
        verify(e.author, &self.signed()?, &self.binding)
    }
    fn change(&self, state: &State) -> Result<Change> {
        let clear = self.envelope.decrypt(
            state
                .keys
                .get(&self.envelope.epoch)
                .ok_or("historical key")?,
        )?;
        let change = Change::from_bytes(clear.to_vec()).map_err(|_| "change")?;
        if change.actor_id().to_bytes() != self.envelope.author {
            return Err("actor identity");
        }
        Ok(change)
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Checkpoint {
    sealed: SealedState,
    author: Id,
    signature: Vec<u8>,
}
impl Checkpoint {
    fn signed(&self) -> Result<Vec<u8>> {
        encode(&("Pass2P/spike/checkpoint/2", &self.sealed, self.author))
    }
    fn create(state: &State, password: &[u8], signer: &SigningKey) -> Result<Self> {
        let mut c = Self {
            sealed: SealedState::seal(state, password)?,
            author: signer.verifying_key().to_bytes(),
            signature: vec![],
        };
        c.signature = signer.sign(&c.signed()?).to_bytes().to_vec();
        Ok(c)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Container {
    version: u16,
    pub generation: u64,
    pub chain: Chain,
    checkpoint: Checkpoint,
    #[serde(default)]
    base: Option<Checkpoint>,
    /// Retain originals, including applied and quarantined packets, across rotation.
    pub queue: Vec<Packet>,
    pub forks: Vec<Chain>,
    archives: Vec<SealedState>,
    retained: Vec<Checkpoint>,
    invitations: Vec<BoundInvitation>,
}
impl Container {
    fn validate(&self, pinned: Id) -> Result<()> {
        if ![2, 3].contains(&self.version) {
            return Err("version");
        }
        self.chain.validate(pinned)?;
        if self.version == 3 {
            let base = self.base.as_ref().ok_or("missing management baseline")?;
            let current = self.chain.head();
            let states = self.chain.states();
            let previous_manager = states[states.len().saturating_sub(2)].manager;
            if base.sealed.control != current.hash()
                || base.sealed.group != current.group
                || base.sealed.epoch != current.epoch
                || (base.author != current.manager && base.author != previous_manager)
            {
                return Err("management baseline binding");
            }
            verify(base.author, &base.signed()?, &base.signature)?;
        }
        let c = &self.checkpoint;
        if c.sealed.control != self.chain.head().hash()
            || c.sealed.group != self.chain.head().group
            || c.sealed.epoch != self.chain.head().epoch
            || !self.chain.head().members.contains(&c.author)
        {
            return Err("checkpoint control");
        }
        verify(c.author, &c.signed()?, &c.signature)?;
        for p in &self.queue {
            p.validate(&self.chain)?;
        }
        for checkpoint in &self.retained {
            let states = self.chain.states();
            let origin = states[self.chain.origin(checkpoint.sealed.control)?];
            if checkpoint.sealed.group != origin.group
                || checkpoint.sealed.epoch != origin.epoch
                || !origin.members.contains(&checkpoint.author)
            {
                return Err("retained binding");
            }
            verify(
                checkpoint.author,
                &checkpoint.signed()?,
                &checkpoint.signature,
            )?;
        }
        for fork in &self.forks {
            fork.validate(pinned)?;
        }
        let branches: Vec<_> = std::iter::once(&self.chain)
            .chain(self.forks.iter())
            .collect();
        if !self.forks.is_empty()
            && !branches
                .iter()
                .any(|a| branches.iter().any(|b| diverges(a, b)))
        {
            return Err("invalid fork evidence");
        }
        Ok(())
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        let bytes = encode(self)?;
        if bytes.len() > MAX_BYTES {
            return Err("size");
        }
        Ok(bytes)
    }
    pub fn parse(bytes: &[u8], pinned: Id) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err("size");
        }
        let c: Self = serde_json::from_slice(bytes).map_err(|_| "container")?;
        if c.version != 3 {
            return Err("container migration required");
        }
        c.validate(pinned)?;
        Ok(c)
    }
    pub fn unlock(&self, password: &[u8]) -> Result<State> {
        let mut state = self.checkpoint.sealed.open(password)?;
        let mut doc = state.doc()?;
        // Only this store's previously committed snapshots enter retained. Incoming
        // retained snapshots are never adopted as acceptance authority.
        for checkpoint in &self.retained {
            let old = checkpoint.sealed.open_key(
                state
                    .keys
                    .get(&checkpoint.sealed.epoch)
                    .ok_or("retained snapshot key")?,
            )?;
            let mut accepted = old.doc()?;
            doc.merge(&mut accepted)
                .map_err(|_| "retained accepted history")?;
            state.applied.extend(old.applied);
        }
        state.document = doc.save();
        Ok(state)
    }
    fn running(&self) -> Result<()> {
        if !self.forks.is_empty() {
            return Err("control fork: exchange halted");
        }
        Ok(())
    }
}
fn diverges(a: &Chain, b: &Chain) -> bool {
    a.states()
        .iter()
        .zip(b.states())
        .any(|(x, y)| x.hash() != y.hash())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    AutoApply,
    NeedsReview,
    AlreadyApplied,
    AwaitingDependencies,
}
struct Candidate {
    change: Change,
    allowed: bool,
}
fn candidates(c: &Container, s: &State) -> Result<BTreeMap<ChangeHash, Candidate>> {
    let mut out = BTreeMap::new();
    for p in &c.queue {
        let change = p.change(s)?;
        let allowed = c.chain.continuous(p.envelope.author, p.control);
        // Among the available signed proofs use the most restrictive admission.
        // A conflicting historical proof cannot be hidden by a second wrapper.
        out.entry(change.hash())
            .and_modify(|v: &mut Candidate| v.allowed &= allowed)
            .or_insert(Candidate { change, allowed });
    }
    Ok(out)
}
fn classify(
    hash: ChangeHash,
    doc: &Automerge,
    pending: &BTreeMap<ChangeHash, Candidate>,
    memo: &mut BTreeMap<ChangeHash, Transfer>,
    visiting: &mut BTreeSet<ChangeHash>,
) -> Result<Transfer> {
    if doc.get_change_by_hash(&hash).is_some() {
        return Ok(Transfer::AlreadyApplied);
    }
    if let Some(result) = memo.get(&hash) {
        return Ok(*result);
    }
    let Some(c) = pending.get(&hash) else {
        return Ok(Transfer::AwaitingDependencies);
    };
    if !visiting.insert(hash) {
        return Err("dependency cycle");
    }
    let mut review = !c.allowed;
    let mut missing = false;
    for dep in c.change.deps() {
        match classify(*dep, doc, pending, memo, visiting)? {
            Transfer::NeedsReview => review = true,
            Transfer::AwaitingDependencies => missing = true,
            _ => {}
        }
    }
    visiting.remove(&hash);
    let result = if missing {
        Transfer::AwaitingDependencies
    } else if review {
        Transfer::NeedsReview
    } else {
        Transfer::AutoApply
    };
    memo.insert(hash, result);
    Ok(result)
}
fn assessment(c: &Container, s: &State) -> Result<BTreeMap<ChangeHash, Transfer>> {
    let doc = s.doc()?;
    let pending = candidates(c, s)?;
    let mut result = BTreeMap::new();
    for hash in pending.keys() {
        let status = classify(*hash, &doc, &pending, &mut result, &mut BTreeSet::new())?;
        result.insert(*hash, status);
    }
    Ok(result)
}

/// No read keys or open document. Every operation reloads the committed generation,
/// including retries after a rename whose directory sync was not acknowledged.
pub struct Store {
    pub path: PathBuf,
    pinned: Id,
    halted: Cell<bool>,
}
impl Store {
    pub fn create(
        path: PathBuf,
        root: Control,
        password: &[u8],
        manager: &SigningKey,
    ) -> Result<Self> {
        if path.exists() {
            return Err("already exists");
        }
        if manager.verifying_key().to_bytes() != root.manager {
            return Err("manager");
        }
        let state = State::empty(&root);
        let checkpoint = Checkpoint::create(&state, password, manager)?;
        let c = Container {
            version: 3,
            generation: 0,
            chain: Chain {
                root,
                successors: vec![],
            },
            checkpoint: checkpoint.clone(),
            base: Some(checkpoint),
            queue: vec![],
            forks: vec![],
            archives: vec![],
            retained: vec![],
            invitations: vec![],
        };
        let store = Self::at(path, c.chain.root.hash());
        c.validate(store.pinned)?;
        atomic_save(&store.path, &c.bytes()?, Fault::None)?;
        crate::storage::initialize(&store.path)?;
        Ok(store)
    }
    pub fn at(path: PathBuf, pinned: Id) -> Self {
        Self {
            path,
            pinned,
            halted: Cell::new(false),
        }
    }
    pub fn load(&self) -> Result<Container> {
        let bytes = crate::storage::read_bounded(&self.path)?;
        crate::storage::check_anchor(&self.path, &bytes)?;
        Container::parse(&bytes, self.pinned)
    }
    fn active(&self) -> Result<Container> {
        if self.halted.get() {
            return Err("control fork: exchange halted");
        }
        let c = self.load()?;
        c.running()?;
        Ok(c)
    }
    fn commit(&self, mut c: Container, fault: Fault) -> Result<()> {
        c.generation = c.generation.checked_add(1).ok_or("generation overflow")?;
        c.validate(self.pinned)?;
        crate::storage::commit(&self.path, c.generation, &c.bytes()?, fault)
    }
    fn author(c: &Container, signer: &SigningKey) -> Result<()> {
        if !c
            .chain
            .head()
            .members
            .contains(&signer.verifying_key().to_bytes())
        {
            return Err("no admission");
        }
        Ok(())
    }
    pub fn export(&self, peer: Id) -> Result<Vec<u8>> {
        let c = self.active()?;
        if !c.chain.head().members.contains(&peer) {
            return Err("no admission");
        }
        c.bytes()
    }
    /// Historical authors may be revoked; only the forwarding peer needs current
    /// admission. Signature validation grants storage, never application.
    pub fn receive(&self, peer: Id, packet: Packet, fault: Fault) -> Result<()> {
        let mut c = self.active()?;
        if !c.chain.head().members.contains(&peer) {
            return Err("no admission");
        }
        packet.validate(&c.chain)?;
        if !c.queue.iter().any(|p| p.id() == packet.id()) {
            c.queue.push(packet);
        }
        self.commit(c, fault)
    }
    /// Receive the latest complete container while locked. Local queued changes
    /// survive snapshot replacement and are reassessed after the current password.
    pub fn receive_container(&self, peer: Id, bytes: &[u8], fault: Fault) -> Result<()> {
        let mut c = self.active()?;
        if !c.chain.head().members.contains(&peer) {
            return Err("no admission");
        }
        let other = Container::parse(bytes, self.pinned)?;
        if diverges(&c.chain, &other.chain) || !other.forks.is_empty() {
            self.halted.set(true);
            if diverges(&c.chain, &other.chain) {
                c.forks.push(other.chain);
            } else {
                c.forks.push(other.chain);
                c.forks.extend(other.forks);
            }
            self.commit(c, fault)?;
            return Err("control fork: exchange halted");
        }
        if !other.chain.head().members.contains(&peer) {
            return Err("no current admission");
        }
        // Equal/older snapshots never overwrite locally accepted content. Changes
        // travel through original packets. Advancing control requires latest state.
        if other.chain.head().seq > c.chain.head().seq {
            c.retained.push(c.checkpoint.clone());
            c.chain = other.chain;
            // Only a management-approved baseline can establish accepted history.
            // A member's later checkpoint must not smuggle revoked dependencies.
            c.checkpoint = other.base.ok_or("missing management baseline")?;
            c.base = Some(c.checkpoint.clone());
        }
        // Keep encrypted recovery sources available when forwarding a container.
        for archive in other.archives {
            if !c
                .archives
                .iter()
                .any(|a| encode(a).ok() == encode(&archive).ok())
            {
                c.archives.push(archive);
            }
        }
        for p in other.queue {
            if !c.queue.iter().any(|ours| ours.id() == p.id()) {
                c.queue.push(p);
            }
        }
        self.commit(c, fault)
    }
    pub fn inspect(&self, password: &[u8]) -> Result<BTreeMap<ChangeHash, Transfer>> {
        let c = self.active()?;
        assessment(&c, &c.unlock(password)?)
    }
    pub fn apply(
        &self,
        password: &[u8],
        signer: &SigningKey,
        fault: Fault,
    ) -> Result<BTreeMap<ChangeHash, Transfer>> {
        let mut c = self.active()?;
        Self::author(&c, signer)?;
        let mut s = c.unlock(password)?;
        let report = assessment(&c, &s)?;
        let pending = candidates(&c, &s)?;
        let mut doc = s.doc()?;
        let changes: Vec<_> = pending
            .into_iter()
            .filter_map(|(h, v)| (report[&h] == Transfer::AutoApply).then_some(v.change))
            .collect();
        // The full transitive closure was checked above, before calling Automerge.
        doc.apply_changes(changes).map_err(|_| "apply")?;
        if !doc.get_missing_deps(&[]).is_empty() {
            return Err("incomplete apply");
        }
        s.applied
            .extend(doc.get_changes(&[]).iter().map(|ch| ch.hash().0));
        s.document = doc.save();
        c.checkpoint = Checkpoint::create(&s, password, signer)?;
        c.retained.clear();
        self.commit(c, fault)?;
        Ok(report)
    }
    pub fn rotate(
        &self,
        old_password: &[u8],
        new_password: &[u8],
        manager: &SigningKey,
        members: BTreeSet<Id>,
        fault: Fault,
    ) -> Result<()> {
        let mut c = self.active()?;
        if old_password == new_password {
            return Err("new password required");
        }
        let mut s = c.unlock(old_password)?;
        if manager.verifying_key().to_bytes() != c.chain.head().manager {
            return Err("manager");
        }
        let previous = c.chain.head();
        let mut next = previous.clone();
        next.seq = next.seq.checked_add(1).ok_or("sequence overflow")?;
        next.epoch = next.epoch.checked_add(1).ok_or("epoch overflow")?;
        next.previous = previous.hash();
        next.members = members;
        let signature = previous.sign_successor(&next, manager, true)?;
        previous.accept(&next, &signature)?;
        let before = self
            .path
            .with_extension(format!("before-rotation-{}", next.seq));
        if !before.exists() {
            atomic_save(&before, &c.bytes()?, Fault::None)?;
        }
        s.epoch = next.epoch;
        s.control = next.hash();
        s.keys.insert(s.epoch, random());
        c.chain.successors.push(SignedControl {
            state: next,
            signature,
        });
        c.checkpoint = Checkpoint::create(&s, new_password, manager)?;
        c.base = Some(c.checkpoint.clone());
        c.retained.clear();
        self.commit(c, fault)
    }
    /// Commit a synthetic string/map edit as one original saved revision.
    pub fn edit(
        &self,
        password: &[u8],
        signer: &SigningKey,
        path: &[String],
        value: &str,
        fault: Fault,
    ) -> Result<ChangeHash> {
        let mut c = self.active()?;
        Self::author(&c, signer)?;
        let mut s = c.unlock(password)?;
        let mut doc = s.doc()?;
        doc.set_actor(actor(signer));
        let mut tx = doc.transaction();
        put_path(&mut tx, path, value)?;
        let (hash, _) = tx.commit();
        let hash = hash.ok_or("empty edit")?;
        c.queue.push(Packet::create(
            &s,
            signer,
            &doc.get_change_by_hash(&hash).ok_or("change")?,
        )?);
        s.applied.insert(hash.0);
        s.document = doc.save();
        c.checkpoint = Checkpoint::create(&s, password, signer)?;
        c.retained.clear();
        self.commit(c, fault)?;
        Ok(hash)
    }
}
fn actor(signer: &SigningKey) -> ActorId {
    ActorId::from(signer.verifying_key().to_bytes().to_vec())
}
fn put_path(tx: &mut Transaction<'_>, path: &[String], value: &str) -> Result<()> {
    if path.first().is_some_and(|p| p == RECEIPTS) {
        return Err("reserved path");
    }
    let (last, parents) = path.split_last().ok_or("empty selection")?;
    let mut obj = ROOT;
    for name in parents {
        let values = tx.get_all(&obj, name.as_str()).map_err(|_| "get")?;
        obj = match values.as_slice() {
            [] => tx
                .put_object(&obj, name.as_str(), ObjType::Map)
                .map_err(|_| "put map")?,
            [(Value::Object(ObjType::Map), id)] => id.clone(),
            _ => return Err("destination requires review"),
        };
    }
    tx.put(&obj, last.as_str(), value).map_err(|_| "put")
}

const RECEIPTS: &str = "_spike_recovery_receipts";
#[derive(Clone, Serialize, Deserialize)]
pub struct Selection {
    pub path: Vec<String>,
    pub variant: usize,
}
#[derive(Serialize, Deserialize)]
struct Provenance {
    operation: Id,
    source: Id,
    selections: Vec<Selection>,
    /// Original author and change hash remain encrypted in the new revision.
    origins: Vec<(Id, Id)>,
}
fn receipt(doc: &Automerge, operation: Id) -> Result<Option<String>> {
    let Some((Value::Object(ObjType::Map), object)) =
        doc.get(ROOT, RECEIPTS).map_err(|_| "receipt")?
    else {
        return Ok(None);
    };
    let values = doc
        .get_all(object, hex(&operation))
        .map_err(|_| "receipt")?;
    match values.as_slice() {
        [] => Ok(None),
        [(Value::Scalar(s), _)] => s.to_str().map(|s| Some(s.to_owned())).ok_or("receipt type"),
        _ => Err("receipt conflict"),
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn add_receipt(tx: &mut Transaction<'_>, operation: Id, provenance: &str) -> Result<()> {
    let obj = match tx.get(ROOT, RECEIPTS).map_err(|_| "receipt")? {
        None => tx
            .put_object(ROOT, RECEIPTS, ObjType::Map)
            .map_err(|_| "receipt")?,
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => return Err("receipt type"),
    };
    tx.put(obj, hex(&operation), provenance)
        .map_err(|_| "receipt")
}
fn selected(doc: &Automerge, selection: &Selection) -> Result<ScalarValue> {
    if selection.path.first().is_some_and(|p| p == RECEIPTS) {
        return Err("reserved path");
    }
    let (last, parents) = selection.path.split_last().ok_or("empty selection")?;
    let mut obj = ROOT;
    for name in parents {
        let values = doc.get_all(&obj, name.as_str()).map_err(|_| "selection")?;
        obj = match values.as_slice() {
            [(Value::Object(ObjType::Map), id)] => id.clone(),
            _ => return Err("ambiguous source path"),
        };
    }
    let values = doc.get_all(obj, last.as_str()).map_err(|_| "selection")?;
    match values.get(selection.variant) {
        Some((Value::Scalar(value), _)) => Ok(value.as_ref().clone()),
        _ => Err("select scalar value"),
    }
}
/// Materialize just the source change's causal view in isolation. This NEVER
/// merges unreviewed dependencies into the current document.
fn review_doc(c: &Container, s: &State, source: ChangeHash) -> Result<Automerge> {
    let pending = candidates(c, s)?;
    let mut doc = s.doc()?;
    fn collect(
        hash: ChangeHash,
        doc: &Automerge,
        pending: &BTreeMap<ChangeHash, Candidate>,
        seen: &mut BTreeSet<ChangeHash>,
        out: &mut Vec<Change>,
    ) -> Result<()> {
        if doc.get_change_by_hash(&hash).is_some() || !seen.insert(hash) {
            return Ok(());
        }
        let c = pending.get(&hash).ok_or("awaiting dependencies")?;
        for dep in c.change.deps() {
            collect(*dep, doc, pending, seen, out)?;
        }
        out.push(c.change.clone());
        Ok(())
    }
    let mut changes = vec![];
    collect(source, &doc, &pending, &mut BTreeSet::new(), &mut changes)?;
    doc.apply_changes(changes).map_err(|_| "review apply")?;
    doc.fork_at(&[source]).map_err(|_| "review view")
}
fn origins(doc: &Automerge) -> Result<Vec<(Id, Id)>> {
    doc.get_changes(&[])
        .iter()
        .map(|ch| {
            Ok((
                ch.actor_id()
                    .to_bytes()
                    .try_into()
                    .map_err(|_| "source actor")?,
                ch.hash().0,
            ))
        })
        .collect()
}
#[derive(Clone)]
enum Tree {
    Map(BTreeMap<String, Tree>),
    Scalar(ScalarValue),
}
fn tree(doc: &Automerge, obj: &ObjId, root: bool) -> Result<Tree> {
    let mut values = BTreeMap::new();
    for key in doc.keys(obj) {
        if root && key == RECEIPTS {
            continue;
        }
        let variants = doc.get_all(obj, key.as_str()).map_err(|_| "backup field")?;
        let value = match variants.as_slice() {
            [(Value::Object(ObjType::Map), id)] => tree(doc, id, false)?,
            [(Value::Scalar(value), _)] => Tree::Scalar(value.as_ref().clone()),
            _ => return Err("backup schema or conflict requires review"),
        };
        values.insert(key, value);
    }
    Ok(Tree::Map(values))
}
fn write_tree(
    tx: &mut Transaction<'_>,
    obj: &ObjId,
    values: &BTreeMap<String, Tree>,
) -> Result<()> {
    for (key, value) in values {
        match value {
            Tree::Scalar(value) => tx
                .put(obj, key.as_str(), value.clone())
                .map_err(|_| "restore scalar")?,
            Tree::Map(children) => {
                let child = tx
                    .put_object(obj, key.as_str(), ObjType::Map)
                    .map_err(|_| "restore map")?;
                write_tree(tx, &child, children)?;
            }
        }
    }
    Ok(())
}
impl Store {
    pub fn review(
        &self,
        password: &[u8],
        source: ChangeHash,
        choices: &[Selection],
    ) -> Result<Vec<ScalarValue>> {
        let c = self.load()?;
        let s = c.unlock(password)?;
        let doc = review_doc(&c, &s, source)?;
        choices
            .iter()
            .map(|choice| selected(&doc, choice))
            .collect()
    }
    /// Explicit user-confirmed extraction. It creates a new author-signed change;
    /// original source hashes are NOT accepted as dependencies by this action.
    pub fn extract(
        &self,
        password: &[u8],
        signer: &SigningKey,
        request: Extraction,
        fault: Fault,
    ) -> Result<ChangeHash> {
        let mut c = self.active()?;
        Self::author(&c, signer)?;
        if request.choices.is_empty() {
            return Err("empty selection");
        }
        let paths: BTreeSet<_> = request.choices.iter().map(|choice| &choice.path).collect();
        if paths.len() != request.choices.len() {
            return Err("choose one variant per path");
        }
        let mut s = c.unlock(password)?;
        let source_doc = review_doc(&c, &s, request.source)?;
        let provenance = Provenance {
            operation: request.operation,
            source: request.source.0,
            selections: request.choices.clone(),
            origins: origins(&source_doc)?,
        };
        let message = String::from_utf8(encode(&provenance)?).map_err(|_| "provenance")?;
        let mut doc = s.doc()?;
        if let Some(previous) = receipt(&doc, request.operation)? {
            if previous != message {
                return Err("operation reused with different selection");
            }
            // Re-sync even on a retry of an uncertain previous rename before success.
            let hash = recovery_hash(&doc, &message)?;
            self.commit(c, fault)?;
            return Ok(hash);
        }
        let values: Vec<_> = request
            .choices
            .iter()
            .map(|ch| selected(&source_doc, ch))
            .collect::<Result<_>>()?;
        doc.set_actor(actor(signer));
        let mut tx = doc.transaction();
        for (choice, value) in request.choices.iter().zip(values) {
            // This scenario schema uses string fields; never silently coerce other values.
            put_path(
                &mut tx,
                &choice.path,
                value.to_str().ok_or("extraction schema")?,
            )?;
        }
        add_receipt(&mut tx, request.operation, &message)?;
        let (hash, _) = tx.commit_with(CommitOptions::default().with_message(&message));
        let hash = hash.ok_or("recovery change")?;
        c.queue.push(Packet::create(
            &s,
            signer,
            &doc.get_change_by_hash(&hash).ok_or("change")?,
        )?);
        s.applied.insert(hash.0);
        s.document = doc.save();
        c.checkpoint = Checkpoint::create(&s, password, signer)?;
        c.retained.clear();
        self.commit(c, fault)?;
        Ok(hash)
    }
    /// Restore only data, under current credentials/control. Caller supplies a
    /// stable operation ID across retries; another intentional restore uses a new ID.
    pub fn restore(
        &self,
        password: &[u8],
        signer: &SigningKey,
        operation: Id,
        backup: &Container,
        fault: Fault,
    ) -> Result<ChangeHash> {
        let mut c = self.active()?;
        Self::author(&c, signer)?;
        backup.validate(self.pinned)?;
        if diverges(&c.chain, &backup.chain) || !backup.forks.is_empty() {
            self.halted.set(true);
            c.forks.push(backup.chain.clone());
            c.forks.extend(backup.forks.clone());
            self.commit(c, fault)?;
            return Err("control fork: exchange halted");
        }
        let mut s = c.unlock(password)?;
        let sealed = &backup.checkpoint.sealed;
        let old = sealed.open_key(s.keys.get(&sealed.epoch).ok_or("backup key")?)?;
        let source_doc = old.doc()?;
        let provenance = Provenance {
            operation,
            source: digest(&encode(sealed)?),
            selections: vec![],
            origins: origins(&source_doc)?,
        };
        let message = String::from_utf8(encode(&provenance)?).map_err(|_| "provenance")?;
        let mut doc = s.doc()?;
        if let Some(previous) = receipt(&doc, operation)? {
            if previous != message {
                return Err("operation reused with different backup");
            }
            let hash = recovery_hash(&doc, &message)?;
            self.commit(c, fault)?;
            return Ok(hash);
        }
        let Tree::Map(values) = tree(&source_doc, &ROOT, true)? else {
            return Err("backup root");
        };
        // A dedicated pre-restore copy cannot be overwritten by the ordinary
        // .previous backup on a subsequent retry. It precedes any mutation.
        let before = self
            .path
            .with_extension(format!("before-restore-{}", hex(&operation)));
        if !before.exists() {
            atomic_save(&before, &c.bytes()?, Fault::None)?;
        }
        doc.set_actor(actor(signer));
        let keys: Vec<_> = doc.keys(ROOT).filter(|k| k != RECEIPTS).collect();
        let mut tx = doc.transaction();
        for key in keys {
            tx.delete(ROOT, key).map_err(|_| "restore delete")?;
        }
        write_tree(&mut tx, &ROOT, &values)?;
        add_receipt(&mut tx, operation, &message)?;
        let (hash, _) = tx.commit_with(CommitOptions::default().with_message(&message));
        let hash = hash.ok_or("restore change")?;
        c.queue.push(Packet::create(
            &s,
            signer,
            &doc.get_change_by_hash(&hash).ok_or("change")?,
        )?);
        c.archives.push(sealed.clone());
        s.applied.insert(hash.0);
        s.document = doc.save();
        c.checkpoint = Checkpoint::create(&s, password, signer)?;
        c.retained.clear();
        self.commit(c, fault)?;
        Ok(hash)
    }
    pub fn recover_trust(
        &self,
        password: &[u8],
        new_password: &[u8],
        manager: &SigningKey,
        new_path: PathBuf,
    ) -> Result<Store> {
        let old = self.load()?.unlock(password)?; // Local reading remains possible after a fork.
        let id = manager.verifying_key().to_bytes();
        let root = Control {
            group: random(),
            seq: 0,
            epoch: 1,
            manager: id,
            members: [id].into(),
            previous: [0; 32],
        };
        if new_path.exists() {
            return Err("already exists");
        }
        let mut s = State::empty(&root);
        s.document = old.document;
        s.applied = old.applied;
        let checkpoint = Checkpoint::create(&s, new_password, manager)?;
        let c = Container {
            version: 3,
            generation: 0,
            chain: Chain {
                root,
                successors: vec![],
            },
            checkpoint: checkpoint.clone(),
            base: Some(checkpoint),
            queue: vec![],
            forks: vec![],
            archives: vec![],
            retained: vec![],
            invitations: vec![],
        };
        let store = Store::at(new_path, c.chain.root.hash());
        c.validate(store.pinned)?;
        atomic_save(&store.path, &c.bytes()?, Fault::None)?;
        crate::storage::initialize(&store.path)?;
        Ok(store)
    }
}
#[derive(Clone)]
pub struct Extraction {
    pub operation: Id,
    pub source: ChangeHash,
    pub choices: Vec<Selection>,
}
fn recovery_hash(doc: &Automerge, message: &str) -> Result<ChangeHash> {
    doc.get_changes(&[])
        .iter()
        .find(|ch| ch.message().is_some_and(|m| m == message))
        .map(|ch| ch.hash())
        .ok_or("missing recovery revision")
}

#[derive(Clone, Serialize, Deserialize)]
struct BoundInvitation {
    group: [u8; 16],
    manager: Id,
    recipient: Id,
    token_hash: Id,
    expires: u64,
    consumed: bool,
}
/// QR encodes this same long random code; QR rendering and transport are deferred with UI.
#[derive(Clone, Serialize, Deserialize)]
pub struct InvitationCode {
    pub group: [u8; 16],
    pub recipient: Id,
    pub token: Id,
}
impl InvitationCode {
    pub fn qr_payload(&self) -> Result<String> {
        String::from_utf8(encode(&(
            "Pass2P/spike/invitation/2",
            hex(&self.group),
            hex(&self.recipient),
            hex(&self.token),
        ))?)
        .map_err(|_| "code")
    }
}
impl Store {
    pub fn invite(
        &self,
        password: &[u8],
        manager: &SigningKey,
        recipient: Id,
        now: u64,
        fault: Fault,
    ) -> Result<InvitationCode> {
        let mut c = self.active()?;
        c.unlock(password)?;
        if manager.verifying_key().to_bytes() != c.chain.head().manager {
            return Err("manager");
        }
        let code = InvitationCode {
            group: c.chain.head().group,
            recipient,
            token: random(),
        };
        c.invitations.push(BoundInvitation {
            group: code.group,
            manager: c.chain.head().manager,
            recipient,
            token_hash: digest(&code.token),
            expires: now.checked_add(300).ok_or("clock overflow")?,
            consumed: false,
        });
        self.commit(c, fault)?;
        Ok(code)
    }
    pub fn redeem(
        &self,
        password: &[u8],
        manager: &SigningKey,
        request: Redemption,
        fault: Fault,
    ) -> Result<()> {
        let mut c = self.active()?;
        let mut s = c.unlock(password)?;
        if manager.verifying_key().to_bytes() != c.chain.head().manager {
            return Err("manager");
        }
        let inv = c
            .invitations
            .iter_mut()
            .find(|i| i.token_hash == digest(&request.code.token))
            .ok_or("invitation")?;
        if inv.consumed
            || request.now >= inv.expires
            || inv.group != request.code.group
            || inv.recipient != request.peer
            || inv.recipient != request.code.recipient
            || inv.manager != manager.verifying_key().to_bytes()
        {
            return Err("invitation invalid");
        }
        inv.consumed = true;
        if request.approved {
            let previous = c.chain.head();
            let mut next = previous.clone();
            next.seq = next.seq.checked_add(1).ok_or("sequence overflow")?;
            next.previous = previous.hash();
            next.members.insert(request.peer);
            let signature = previous.sign_successor(&next, manager, true)?;
            previous.accept(&next, &signature)?;
            s.control = next.hash();
            c.chain.successors.push(SignedControl {
                state: next,
                signature,
            });
            c.checkpoint = Checkpoint::create(&s, password, manager)?;
            c.base = Some(c.checkpoint.clone());
            c.retained.clear();
        }
        // Consumed token and membership are committed in the SAME generation.
        self.commit(c, fault)?;
        if !request.approved {
            return Err("manager denied");
        }
        Ok(())
    }
}
pub struct Redemption {
    pub code: InvitationCode,
    pub peer: Id,
    pub now: u64,
    pub approved: bool,
}

#[cfg(test)]
mod tests;

impl Store {
    /// Management transition persists the calibrated parameters in the signed header.
    pub fn calibrate_kdf(
        &self,
        password: &[u8],
        manager: &SigningKey,
        target_ms: u64,
    ) -> Result<crate::kdf::Calibration> {
        let mut c = self.active()?;
        let mut s = c.unlock(password)?;
        if c.chain.head().manager != manager.verifying_key().to_bytes() {
            return Err("manager");
        }
        let result = crate::kdf::calibrate(target_ms)?;
        if !result.within_tolerance {
            return Err("KDF target not reached");
        }
        let previous = c.chain.head();
        let mut next = previous.clone();
        next.seq = next.seq.checked_add(1).ok_or("sequence overflow")?;
        next.previous = previous.hash();
        let signature = previous.sign_successor(&next, manager, true)?;
        previous.accept(&next, &signature)?;
        s.control = next.hash();
        s.kdf = result.profile;
        c.chain.successors.push(SignedControl {
            state: next,
            signature,
        });
        c.checkpoint = Checkpoint::create(&s, password, manager)?;
        c.base = Some(c.checkpoint.clone());
        c.retained.clear();
        self.commit(c, Fault::None)?;
        Ok(result)
    }
}

/// Recipient signs an explicit unlocked acceptance before the old manager relinquishes.
#[derive(Clone, Serialize, Deserialize)]
pub struct HandoffConsent {
    previous: Id,
    recipient: Id,
    signature: Vec<u8>,
}
impl HandoffConsent {
    pub fn sign(previous: &Control, recipient: &SigningKey, unlocked: bool) -> Result<Self> {
        let id = recipient.verifying_key().to_bytes();
        if !unlocked || !previous.members.contains(&id) || id == previous.manager {
            return Err("handoff recipient");
        }
        let mut consent = Self {
            previous: previous.hash(),
            recipient: id,
            signature: vec![],
        };
        consent.signature = recipient.sign(&consent.signed()?).to_bytes().to_vec();
        Ok(consent)
    }
    fn signed(&self) -> Result<Vec<u8>> {
        encode(&(
            "Pass2P/spike/handoff-consent/2",
            self.previous,
            self.recipient,
        ))
    }
}
impl Store {
    pub fn handoff(
        &self,
        password: &[u8],
        manager: &SigningKey,
        consent: &HandoffConsent,
        fault: Fault,
    ) -> Result<()> {
        let mut c = self.active()?;
        let mut s = c.unlock(password)?;
        verify(consent.recipient, &consent.signed()?, &consent.signature)?;
        // The same durable relinquishment is retried after a lost response.
        if c.chain.head().previous == consent.previous
            && c.chain.head().manager == consent.recipient
        {
            return self.commit(c, fault);
        }
        if c.chain.head().hash() != consent.previous {
            return Err("stale handoff");
        }
        let previous = c.chain.head();
        let mut next = previous.clone();
        next.seq = next.seq.checked_add(1).ok_or("sequence overflow")?;
        next.previous = previous.hash();
        next.manager = consent.recipient;
        let signature = previous.sign_successor(&next, manager, true)?;
        previous.accept(&next, &signature)?;
        s.control = next.hash();
        c.chain.successors.push(SignedControl {
            state: next,
            signature,
        });
        c.checkpoint = Checkpoint::create(&s, password, manager)?;
        c.base = Some(c.checkpoint.clone());
        c.retained.clear();
        self.commit(c, fault)
    }
}

/// Local encrypted draft. It is deliberately not a field of the portable container.
#[derive(Serialize, Deserialize)]
pub struct Draft {
    group: [u8; 16],
    epoch: u64,
    content: Ciphertext,
}
impl Draft {
    fn aad(&self) -> Result<Vec<u8>> {
        encode(&("Pass2P/spike/local-draft/2", self.group, self.epoch))
    }
    pub fn save(state: &State, clear: &[u8], path: &Path, fault: Fault) -> Result<()> {
        if clear.len() > MAX_BYTES / 8 {
            return Err("draft size");
        }
        let mut draft = Self {
            group: state.group,
            epoch: state.epoch,
            content: Ciphertext {
                nonce: [0; 24],
                bytes: vec![],
            },
        };
        draft.content = Ciphertext::seal(
            state.keys.get(&state.epoch).ok_or("draft key")?,
            &draft.aad()?,
            clear,
        )?;
        atomic_save(path, &encode(&draft)?, fault)
    }
    pub fn open(state: &State, path: &Path) -> Result<Zeroizing<Vec<u8>>> {
        let draft: Self =
            serde_json::from_slice(&crate::storage::read_bounded(path)?).map_err(|_| "draft")?;
        if draft.group != state.group {
            return Err("draft group");
        }
        draft.content.open(
            state.keys.get(&draft.epoch).ok_or("draft key")?,
            &draft.aad()?,
        )
    }
}

impl Store {
    /// Explicit local migration with management approval; never performed on network input.
    pub fn migrate_v2(&self, password: &[u8], manager: &SigningKey, fault: Fault) -> Result<()> {
        let bytes = crate::storage::read_bounded(&self.path)?;
        crate::storage::check_anchor(&self.path, &bytes)?;
        let mut c: Container = serde_json::from_slice(&bytes).map_err(|_| "legacy container")?;
        if c.version != 2 || c.base.is_some() {
            return Err("migration version");
        }
        c.validate(self.pinned)?;
        if manager.verifying_key().to_bytes() != c.chain.head().manager {
            return Err("manager");
        }
        c.running()?;
        let state = c.unlock(password)?;
        let backup = self.path.with_extension("before-migration-2");
        if !backup.exists() {
            atomic_save(&backup, &bytes, Fault::None)?;
        }
        c.checkpoint = Checkpoint::create(&state, password, manager)?;
        c.base = Some(c.checkpoint.clone());
        c.retained.clear();
        c.version = 3;
        self.commit(c, fault)
    }
}
