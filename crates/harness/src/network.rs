use crate::TestPeer;
use openprod_storage::StorageError;

pub struct TestNetwork {
    peers: Vec<TestPeer>,
}

impl Default for TestNetwork {
    fn default() -> Self {
        Self::new()
    }
}

impl TestNetwork {
    pub fn new() -> Self {
        Self { peers: Vec::new() }
    }

    pub fn add_peer(&mut self) -> Result<usize, StorageError> {
        let peer = TestPeer::new()?;
        let index = self.peers.len();
        self.peers.push(peer);
        Ok(index)
    }

    pub fn peer(&self, index: usize) -> &TestPeer {
        &self.peers[index]
    }

    pub fn peer_mut(&mut self, index: usize) -> &mut TestPeer {
        &mut self.peers[index]
    }
}
