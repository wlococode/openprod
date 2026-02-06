use openprod_core::{
    field_value::FieldValue,
    hlc::Hlc,
    ids::*,
    operations::OperationPayload,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlaySource {
    User,
    Script,
}

impl OverlaySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Script => "script",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayStatus {
    Active,
    Stashed,
    Committed,
    Discarded,
}

impl OverlayStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Stashed => "stashed",
            Self::Committed => "committed",
            Self::Discarded => "discarded",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OverlayRecord {
    pub overlay_id: OverlayId,
    pub display_name: String,
    pub source: OverlaySource,
    pub status: OverlayStatus,
    pub created_at: Hlc,
    pub updated_at: Hlc,
}

#[derive(Debug, Clone)]
pub struct OverlayOpRecord {
    pub rowid: i64,
    pub overlay_id: OverlayId,
    pub op_id: OpId,
    pub hlc: Hlc,
    pub payload: OperationPayload,
    pub entity_id: Option<EntityId>,
    pub field_key: Option<String>,
    pub op_type: String,
    pub canonical_value_at_creation: Option<Vec<u8>>,
    pub canonical_drifted: bool,
}

#[derive(Debug, Clone)]
pub struct DriftRecord {
    pub entity_id: EntityId,
    pub field_key: String,
    pub overlay_value: Option<FieldValue>,
    pub canonical_value: Option<FieldValue>,
}

/// Manages overlay lifecycle and in-memory state.
/// Overlay undo/redo is non-persistent (cleared on restart per spec).
pub struct OverlayManager {
    active_overlay_id: Option<OverlayId>,
    /// In-memory undo stack for the active overlay (op records removed from overlay_ops).
    overlay_undo_stack: Vec<OverlayOpRecord>,
    /// In-memory redo stack for the active overlay.
    overlay_redo_stack: Vec<OverlayOpRecord>,
}

impl Default for OverlayManager {
    fn default() -> Self {
        Self::new()
    }
}

impl OverlayManager {
    pub fn new() -> Self {
        Self {
            active_overlay_id: None,
            overlay_undo_stack: Vec::new(),
            overlay_redo_stack: Vec::new(),
        }
    }

    pub fn active_overlay_id(&self) -> Option<OverlayId> {
        self.active_overlay_id
    }

    pub fn set_active(&mut self, overlay_id: Option<OverlayId>) {
        if self.active_overlay_id != overlay_id {
            // Clear overlay undo/redo when switching overlays
            self.overlay_undo_stack.clear();
            self.overlay_redo_stack.clear();
        }
        self.active_overlay_id = overlay_id;
    }

    pub fn push_overlay_undo(&mut self, op: OverlayOpRecord) {
        self.overlay_undo_stack.push(op);
        self.overlay_redo_stack.clear();
    }

    pub fn pop_overlay_undo(&mut self) -> Option<OverlayOpRecord> {
        self.overlay_undo_stack.pop()
    }

    pub fn push_overlay_redo(&mut self, op: OverlayOpRecord) {
        self.overlay_redo_stack.push(op);
    }

    pub fn pop_overlay_redo(&mut self) -> Option<OverlayOpRecord> {
        self.overlay_redo_stack.pop()
    }
}
