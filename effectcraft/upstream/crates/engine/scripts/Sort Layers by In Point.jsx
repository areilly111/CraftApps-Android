// Sort Layers by In Point — an EffectCraft sample script (original work, MIT OR Apache-2.0).
//
// Reorders the selected layers of the active composition (all layers when none are selected)
// so the layer that starts first is on top. Layers with the same in point keep their order.
// One undo step.

(function sortLayersByInPoint() {
  var comp = app.project.activeItem;
  if (!(comp instanceof CompItem)) {
    alert("Sort Layers by In Point: open a composition first.");
    return;
  }
  var layers = comp.selectedLayers;
  if (layers.length === 0) {
    layers = [];
    for (var i = 1; i <= comp.numLayers; i++) layers.push(comp.layer(i));
  }
  if (layers.length < 2) return;

  // The slots the layers occupy now (stack indices, top first).
  var slots = layers.map(function (l) { return l.index; }).sort(function (a, b) { return a - b; });
  var order = layers
    .map(function (l, k) { return { layer: l, inPoint: l.inPoint, k: k }; })
    .sort(function (a, b) { return a.inPoint - b.inPoint || a.k - b.k; });

  app.beginUndoGroup("Sort Layers by In Point");
  // Place each layer, earliest first, in the next slot from the top.
  for (var j = 0; j < order.length; j++) {
    var target = slots[j];
    var layer = order[j].layer;
    if (layer.index === target) continue;
    var anchor = comp.layer(target);
    layer.moveBefore(anchor);
  }
  app.endUndoGroup();
  writeLn("Sorted " + layers.length + " layer(s) by in point.");
})();
