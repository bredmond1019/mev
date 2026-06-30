use crate::brain::manifest::Manifest;
use std::collections::HashMap;
use std::path::Path;

#[derive(serde::Serialize)]
struct VisNode {
    id: String,
    label: String,
    group: String,
    value: usize,
    title: String,
}

#[derive(serde::Serialize)]
struct VisEdge {
    from: String,
    to: String,
}

pub fn generate_graph_visual(manifest: &Manifest, out_dir: &Path) -> anyhow::Result<()> {
    // 1. Pre-process to build node catalog
    let mut node_degrees: HashMap<String, usize> = HashMap::new();
    let mut edges = Vec::new();

    // 2. Pass edges
    for entry in &manifest.entries {
        if let Some(doc_id) = &entry.doc_id {
            let node_id = format!("{}:{}", entry.scope, doc_id);
            if let Some(related) = &entry.related {
                for r in related {
                    let target = if r.contains(':') {
                        r.to_string()
                    } else {
                        format!("{}:{}", entry.scope, r)
                    };
                    edges.push(VisEdge { from: node_id.clone(), to: target.clone() });
                    *node_degrees.entry(node_id.clone()).or_insert(0) += 1;
                    *node_degrees.entry(target.clone()).or_insert(0) += 1;
                }
            }
        }
    }

    let mut vis_nodes = Vec::new();
    for entry in &manifest.entries {
        if let Some(doc_id) = &entry.doc_id {
            let node_id = format!("{}:{}", entry.scope, doc_id);
            if let Some(&degree) = node_degrees.get(&node_id) {
                if degree > 0 {
                    let raw_title = entry.title.clone().unwrap_or_else(|| doc_id.clone());
                    let short_label = if raw_title.len() <= 30 {
                        raw_title.clone()
                    } else {
                        format!("{}...", &raw_title[..27])
                    };
                    let doc_type = entry.doc_type.clone().unwrap_or_else(|| "Document".to_string());
                    
                    vis_nodes.push(VisNode {
                        id: node_id.clone(),
                        label: short_label,
                        group: entry.scope.clone(),
                        value: degree,
                        title: format!("<div style='padding:5px; max-width: 300px;'><b>{}</b><br><i>{}</i><br>ID: {}</div>", raw_title, doc_type, node_id),
                    });
                    
                    // Remove from node_degrees so we can process implicit targets
                    node_degrees.remove(&node_id);
                }
            }
        }
    }
    
    // Add missing target nodes
    for (node_id, degree) in node_degrees {
        if degree > 0 {
            let scope = node_id.split(':').next().unwrap_or("unknown").to_string();
            let label = node_id.split(':').last().unwrap_or(&node_id).to_string();
            vis_nodes.push(VisNode {
                id: node_id.clone(),
                label: label.clone(),
                group: scope,
                value: degree,
                title: format!("<div style='padding:5px; max-width: 300px;'><b>{}</b><br><i>Unknown</i><br>ID: {}</div>", label, node_id),
            });
        }
    }

    let nodes_json = serde_json::to_string(&vis_nodes)?;
    let edges_json = serde_json::to_string(&edges)?;
    let node_count = vis_nodes.len();
    let edge_count = edges.len();

    let md_content = format!(r#"---
type: Reference
title: Brain Knowledge Graph Visual
description: "Interactive visualization of the scope:doc_id nodes and their related: edges across the portfolio."
doc_id: brain-graph-visual
layer: [meta]
project: mev
status: active
---

# Brain Knowledge Graph Visual

The knowledge graph is too large to render via Markdown ({node_count} nodes, {edge_count} edges). Instead, an **interactive HTML graph** has been generated.

Open the `graph.html` file in this directory in any web browser to explore the graph. You can zoom, pan, and drag nodes to see their relationships.
"#);

    let html_content = format!(r#"<!DOCTYPE html>
<html>
<head>
    <title>Brain Knowledge Graph</title>
    <script type="text/javascript" src="https://unpkg.com/vis-network/standalone/umd/vis-network.min.js"></script>
    <style type="text/css">
        body {{ margin: 0; padding: 0; background-color: #0f1115; color: #e1e4e8; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; }}
        #mynetwork {{ width: 100vw; height: 100vh; border: none; }}
        #ui-panel {{ position: absolute; top: 20px; left: 20px; z-index: 10; background: rgba(22, 27, 34, 0.85); padding: 20px; border-radius: 12px; border: 1px solid #30363d; backdrop-filter: blur(10px); box-shadow: 0 8px 24px rgba(0,0,0,0.5); max-width: 300px; }}
        h2 {{ margin: 0 0 10px 0; font-size: 18px; color: #58a6ff; }}
        p {{ margin: 5px 0; font-size: 13px; color: #8b949e; line-height: 1.4; }}
        input, select, button {{ width: 100%; margin-top: 10px; padding: 8px; background: #0d1117; border: 1px solid #30363d; color: #e1e4e8; border-radius: 6px; box-sizing: border-box; }}
        button {{ background: #238636; border: 1px solid rgba(240, 246, 252, 0.1); font-weight: 600; cursor: pointer; transition: 0.2s; }}
        button:hover {{ background: #2ea043; }}
        .vis-tooltip {{ background-color: #161b22 !important; border: 1px solid #30363d !important; color: #e1e4e8 !important; border-radius: 6px !important; box-shadow: 0 4px 12px rgba(0,0,0,0.5) !important; font-family: inherit !important; }}
    </style>
</head>
<body>
<div id="ui-panel">
    <h2>Bastion Brain Graph</h2>
    <p>Nodes: <b>{node_count}</b> &nbsp;|&nbsp; Edges: <b>{edge_count}</b></p>
    <p>Hub nodes are sized larger. Colors denote repository scope. Hover over a node for details.</p>
    
    <div style="margin-top: 15px;">
        <input type="text" id="searchInput" placeholder="Search by Title or ID...">
        <button onclick="searchNode()">Focus Node</button>
    </div>
    
    <div style="margin-top: 10px;">
        <select id="scopeFilter" onchange="filterScope()">
            <option value="">-- Show All Scopes --</option>
        </select>
    </div>
    
    <div style="margin-top: 10px;">
        <button onclick="resetHighlight()" style="background: #21262d;">Reset View</button>
    </div>
</div>
<div id="mynetwork"></div>
<script type="text/javascript">
    var rawNodes = {nodes_json};
    var rawEdges = {edges_json};
    
    var nodes = new vis.DataSet(rawNodes);
    var edges = new vis.DataSet(rawEdges);
    var container = document.getElementById('mynetwork');
    var data = {{ nodes: nodes, edges: edges }};
    
    // A beautiful, modern color palette for different scopes
    var colorPalette = [
        '#58a6ff', '#3fb950', '#d2a8ff', '#f0883e', '#ff7b72', 
        '#79c0ff', '#56d364', '#e3b341', '#ffa198', '#bc8cff'
    ];
    
    var options = {{
        nodes: {{
            shape: 'dot',
            font: {{ color: '#c9d1d9', size: 14, face: 'sans-serif' }},
            borderWidth: 2,
            scaling: {{ min: 10, max: 50, label: {{ enabled: true, min: 14, max: 24 }} }},
            shadow: {{ enabled: true, color: 'rgba(0,0,0,0.8)', size: 10, x: 2, y: 2 }}
        }},
        edges: {{
            color: {{ color: '#484f58', highlight: '#8b949e', hover: '#8b949e' }},
            arrows: {{ to: {{ enabled: true, scaleFactor: 0.5 }} }},
            smooth: {{ type: 'continuous' }}
        }},
        groups: {{}},
        physics: {{
            barnesHut: {{ gravitationalConstant: -8000, centralGravity: 0.1, springLength: 250, springConstant: 0.04 }},
            minVelocity: 0.75,
            solver: 'barnesHut'
        }},
        interaction: {{ hover: true, tooltipDelay: 50, zoomView: true }}
    }};
    
    // Auto-assign colors to groups and populate dropdown
    var groups = [...new Set(rawNodes.map(n => n.group))].sort();
    var select = document.getElementById('scopeFilter');
    groups.forEach((g, idx) => {{
        var color = colorPalette[idx % colorPalette.length];
        options.groups[g] = {{
            color: {{
                background: color,
                border: '#ffffff',
                highlight: {{ background: '#ffffff', border: color }},
                hover: {{ background: '#ffffff', border: color }}
            }}
        }};
        
        var opt = document.createElement('option');
        opt.value = g;
        opt.innerHTML = g;
        select.appendChild(opt);
    }});
    
    var network = new vis.Network(container, data, options);
    
    function searchNode() {{
        var term = document.getElementById('searchInput').value.toLowerCase();
        if (!term) return resetHighlight();
        
        var matches = rawNodes.filter(n => n.id.toLowerCase().includes(term) || (n.title && n.title.toLowerCase().includes(term)));
        if (matches.length > 0) {{
            network.focus(matches[0].id, {{ scale: 1.2, animation: {{ duration: 500, easingFunction: 'easeInOutQuad' }} }});
            network.selectNodes([matches[0].id]);
        }}
    }}
    
    function filterScope() {{
        var scope = document.getElementById('scopeFilter').value;
        if (!scope) {{
            nodes.update(rawNodes.map(n => ({{id: n.id, hidden: false}})));
            edges.update(rawEdges.map(e => ({{id: e.from, hidden: false}}))); // Just triggering update
            return;
        }}
        
        // Hide nodes not in scope
        nodes.update(rawNodes.map(n => ({{id: n.id, hidden: n.group !== scope}})));
    }}
    
    function resetHighlight() {{
        document.getElementById('searchInput').value = '';
        document.getElementById('scopeFilter').value = '';
        nodes.update(rawNodes.map(n => ({{id: n.id, hidden: false}})));
        network.unselectAll();
        network.fit({{ animation: {{ duration: 500, easingFunction: 'easeInOutQuad' }} }});
    }}
</script>
</body>
</html>"#);

    std::fs::create_dir_all(out_dir)?;
    std::fs::write(out_dir.join("graph.md"), md_content)?;
    std::fs::write(out_dir.join("graph.html"), html_content)?;
    
    println!("Graph successfully generated with {node_count} connected nodes and {edge_count} edges.");
    println!("-> {}", out_dir.join("graph.md").display());
    println!("-> {}", out_dir.join("graph.html").display());
    Ok(())
}
