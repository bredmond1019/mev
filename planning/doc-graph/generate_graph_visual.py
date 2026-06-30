#!/usr/bin/env python3
import json
import os
import subprocess

def generate_graph():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    portfolio_root = os.path.abspath(os.path.join(script_dir, "../../../../"))
    mev_path = os.path.abspath(os.path.join(script_dir, "../../target/release/mev"))
    
    print(f"Running mev manifest from {mev_path}...")
    
    result = subprocess.run([mev_path, "manifest", portfolio_root], capture_output=True, text=True)
    if result.returncode != 0:
        print(f"Failed to run mev manifest. Ensure it is built. Error: {result.stderr}")
        return

    data = json.loads(result.stdout)
    edges = []
    
    # Store rich node info
    node_info = {}
    
    # Pre-process entries to build node catalog
    for entry in data.get("entries", []):
        doc_id = entry.get("doc_id")
        scope = entry.get("scope", "unknown")
        if doc_id:
            node_id = f"{scope}:{doc_id}"
            node_info[node_id] = {
                "id": node_id,
                "label": doc_id,
                "scope": scope,
                "title": entry.get("title") or doc_id,
                "doc_type": entry.get("doc_type") or "Document",
                "degree": 0
            }

    # First pass over edges
    for entry in data.get("entries", []):
        doc_id = entry.get("doc_id")
        scope = entry.get("scope", "unknown")
        if doc_id:
            node_id = f"{scope}:{doc_id}"
            related = entry.get("related") or []
            for r in related:
                target = r if ":" in r else f"{scope}:{r}"
                edges.append({"from": node_id, "to": target})
                
                # Ensure target exists in node_info even if it wasn't in the manifest
                if target not in node_info:
                    target_scope = target.split(":")[0] if ":" in target else "unknown"
                    node_info[target] = {
                        "id": target,
                        "label": target.split(":")[-1],
                        "scope": target_scope,
                        "title": target,
                        "doc_type": "Unknown",
                        "degree": 0
                    }
                
                # Increment degree for sizing
                node_info[node_id]["degree"] += 1
                node_info[target]["degree"] += 1

    # Filter out isolated nodes
    connected_nodes = [n for n in node_info.values() if n["degree"] > 0]
    
    # Prepare vis.js data
    vis_nodes = []
    for n in connected_nodes:
        # Create a shorter label from title
        title_str = str(n['title'])
        short_label = title_str if len(title_str) <= 30 else title_str[:27] + "..."
        vis_nodes.append({
            "id": n["id"],
            "label": short_label,
            "group": n["scope"],
            "value": n["degree"],
            "title": f"<div style='padding:5px; max-width: 300px;'><b>{n['title']}</b><br><i>{n['doc_type']}</i><br>ID: {n['id']}</div>"
        })

    md_path = os.path.join(script_dir, "graph.md")
    html_path = os.path.join(script_dir, "graph.html")
    
    with open(md_path, "w") as f:
        f.write("---\n")
        f.write("type: Reference\n")
        f.write("title: Brain Knowledge Graph Visual\n")
        f.write("description: \"Interactive visualization of the scope:doc_id nodes and their related: edges across the portfolio.\"\n")
        f.write("doc_id: brain-graph-visual\n")
        f.write("layer: [meta]\n")
        f.write("project: mev\n")
        f.write("status: active\n")
        f.write("---\n\n")
        
        f.write("# Brain Knowledge Graph Visual\n\n")
        f.write(f"The knowledge graph is too large to render via Markdown ({len(connected_nodes)} nodes, {len(edges)} edges). ")
        f.write("Instead, an **interactive HTML graph** has been generated.\n\n")
        f.write("Open the `graph.html` file in this directory in any web browser to explore the graph. You can zoom, pan, and drag nodes to see their relationships.\n")

    html_content = f"""<!DOCTYPE html>
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
    <p>Nodes: <b>{len(connected_nodes)}</b> &nbsp;|&nbsp; Edges: <b>{len(edges)}</b></p>
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
    var rawNodes = {json.dumps(vis_nodes)};
    var rawEdges = {json.dumps(edges)};
    
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
</html>"""

    with open(html_path, "w") as f:
        f.write(html_content)

    print(f"Graph successfully generated with {len(connected_nodes)} connected nodes and {len(edges)} edges.")
    print(f"-> {md_path}")
    print(f"-> {html_path}")

if __name__ == "__main__":
    generate_graph()
