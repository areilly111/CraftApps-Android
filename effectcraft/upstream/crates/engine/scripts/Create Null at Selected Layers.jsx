// Create Null at Selected Layers — an EffectCraft sample script (original work, MIT OR
// Apache-2.0).
//
// Adds a null at the centroid of the selected layers' positions (at the current time) and
// parents the layers that have no parent to it (set PARENT below to false to skip that).
// One undo step.

(function createNullAtCentroid() {
  var PARENT = true;
  var comp = app.project.activeItem;
  if (!(comp instanceof CompItem)) {
    alert("Create Null: open a composition first.");
    return;
  }
  var layers = comp.selectedLayers;
  if (layers.length === 0) {
    alert("Create Null: select one or more layers.");
    return;
  }
  var t = comp.time;
  var sum = [0, 0, 0];
  for (var i = 0; i < layers.length; i++) {
    var p = layers[i].transform.position.valueAtTime(t, false);
    sum[0] += p[0];
    sum[1] += p[1];
    sum[2] += p.length > 2 ? p[2] : 0;
  }
  var c = [sum[0] / layers.length, sum[1] / layers.length, sum[2] / layers.length];

  app.beginUndoGroup("Create Null at Selected Layers");
  var nul = comp.layers.addNull();
  nul.name = "Centroid";
  nul.transform.position.setValue(c);
  if (PARENT) {
    for (var k = 0; k < layers.length; k++) {
      if (!layers[k].parent) layers[k].setParentWithJump(nul);
    }
  }
  app.endUndoGroup();
  writeLn("Null at [" + c[0].toFixed(1) + ", " + c[1].toFixed(1) + "]");
})();
