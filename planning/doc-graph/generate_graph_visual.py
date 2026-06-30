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
    nodes = set()

    for entry in data.get("entries", []):
        doc_id = entry.get("doc_id")
        scope = entry.get("scope", "unknown")
        if doc_id:
            node_id = f"{scope}:{doc_id}"
            # The related field is flattened
            related = entry.get("related") or []
            for r in related:
                target = r if ":" in r else f"{scope}:{r}"
                edges.append((node_id, target))
                nodes.add(node_id)
                nodes.add(target)

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
        f.write(f"The knowledge graph is too large to render via Markdown ({len(nodes)} nodes, {len(edges)} edges). ")
        f.write("Instead, an **interactive HTML graph** has been generated.\n\n")
        f.write("Open the `graph.html` file in this directory in any web browser to explore the graph. You can zoom, pan, and drag nodes to see their relationships.\n")

    # Generate vis.js interactive HTML
    vis_nodes = [{"id": n, "label": n} for n in sorted(nodes)]
    vis_edges = [{"from": src, "to": dst} for src, dst in sorted(edges)]
    
    html_content = f"""<!DOCTYPE html>
<html>
<head>
    <title>Brain Knowledge Graph</title>
    <script type="text/javascript" src="https://unpkg.com/vis-network/standalone/umd/vis-network.min.js"></script>
    <style type="text/css">
        body {{ margin: 0; padding: 0; background-color: #1a1a1a; color: #fff; font-family: sans-serif; }}
        #mynetwork {{ width: 100vw; height: 100vh; border: none; }}
        #info {{ position: absolute; top: 10px; left: 10px; z-index: 10; background: rgba(0,0,0,0.7); padding: 10px; border-radius: 5px; }}
    </style>
</head>
<body>
<div id="info">
    <h2>Brain Knowledge Graph</h2>
    <p>Nodes: {len(nodes)} | Edges: {len(edges)}</p>
    <p>Scroll to zoom, drag to pan. Drag nodes to move them.</p>
</div>
<div id="mynetwork"></div>
<script type="text/javascript">
    var nodes = new vis.DataSet({json.dumps(vis_nodes)});
    var edges = new vis.DataSet({json.dumps(vis_edges)});
    var container = document.getElementById('mynetwork');
    var data = {{ nodes: nodes, edges: edges }};
    var options = {{
        nodes: {{
            shape: 'dot',
            size: 16,
            font: {{ color: '#ffffff', size: 14 }},
            borderWidth: 2,
            color: {{ background: '#007BFF', border: '#0056b3' }}
        }},
        edges: {{
            color: {{ color: '#666666', highlight: '#ffffff' }},
            arrows: 'to',
            smooth: {{ type: 'continuous' }}
        }},
        physics: {{
            barnesHut: {{ gravitationalConstant: -3000, centralGravity: 0.3, springLength: 95, springConstant: 0.04 }},
            minVelocity: 0.75
        }},
        interaction: {{ hover: true }}
    }};
    var network = new vis.Network(container, data, options);
</script>
</body>
</html>"""

    with open(html_path, "w") as f:
        f.write(html_content)

    print(f"Graph successfully generated with {len(nodes)} connected nodes and {len(edges)} edges.")
    print(f"-> {md_path}")
    print(f"-> {html_path}")

if __name__ == "__main__":
    generate_graph()
