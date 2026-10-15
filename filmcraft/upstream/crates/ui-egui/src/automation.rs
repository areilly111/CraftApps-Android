//! Automation registry: every interactive element registers a stable id and its screen rect each
//! frame, so agents can click/drag by id (`ui.click {id:"timeline.track.V1.lock"}`) and
//! `ui.inspect` can report the full on-screen widget tree.

use egui::Rect;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Element {
    pub id: String,
    pub label: String,
    pub rect: [f32; 4],
    /// Present only when this entry represents a real egui widget, rather than a click alias.
    #[serde(rename = "widgetId", skip_serializing_if = "Option::is_none")]
    pub widget_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focusable: Option<bool>,
    #[serde(skip)]
    pub(crate) response_id: Option<egui::Id>,
}

#[derive(Default)]
pub struct Registry {
    pub elements: Vec<Element>,
    /// Last frame's elements (for lookups from control handlers between frames).
    pub previous: Vec<Element>,
}

impl Registry {
    pub fn add(&mut self, id: &str, rect: Rect, label: &str) {
        self.elements.push(Element {
            id: id.to_string(),
            label: label.to_string(),
            rect: [rect.min.x, rect.min.y, rect.width(), rect.height()],
            widget_id: None,
            enabled: None,
            focusable: None,
            response_id: None,
        });
    }
    /// Observe the widget without changing its accessibility role or input behavior.
    pub fn add_response(&mut self, id: &str, response: &egui::Response, label: &str) {
        self.add(id, response.rect, label);
        if let Some(element) = self.elements.last_mut() {
            element.widget_id = Some(format!("{:016x}", response.id.value()));
            element.enabled = Some(response.enabled());
            element.focusable = Some(response.sense.is_focusable());
            element.response_id = Some(response.id);
        }
    }
    pub fn begin_frame(&mut self) {
        self.previous = std::mem::take(&mut self.elements);
    }
    pub fn find(&self, id: &str) -> Option<&Element> {
        self.previous.iter().chain(self.elements.iter()).find(|e| e.id == id)
    }
    /// Elements whose id starts with a prefix.
    pub fn query(&self, prefix: &str) -> Vec<&Element> {
        self.previous.iter().filter(|e| e.id.starts_with(prefix)).collect()
    }
}
