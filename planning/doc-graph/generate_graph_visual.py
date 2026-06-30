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

    out_path = os.path.join(script_dir, "graph.md")
    
    with open(out_path, "w") as f:
        f.write("---\n")
        f.write("type: Reference\n")
        f.write("title: Brain Knowledge Graph Visual\n")
        f.write("description: A Mermaid diagram visualizing the scope:doc_id nodes and their related: edges across the portfolio.\n")
        f.write("doc_id: brain-graph-visual\n")
        f.write("layer: [meta]\n")
        f.write("project: mev\n")
        f.write("status: active\n")
        f.write("---\n\n")
        
        f.write("# Brain Knowledge Graph Visual\n\n")
        f.write("This diagram visualizes the `scope:doc_id` nodes and their `related:` edges across the entire portfolio (showing only connected nodes for clarity).\n\n")
        f.write("```mermaid\n")
        f.write("graph TD\n")
        for n in sorted(nodes):
            safe_id = n.replace(":", "_").replace("-", "_")
            f.write(f"    {safe_id}[\"{n}\"]\n")
        for src, dst in sorted(edges):
            s_safe = src.replace(":", "_").replace("-", "_")
            d_safe = dst.replace(":", "_").replace("-", "_")
            f.write(f"    {s_safe} --> {d_safe}\n")
        f.write("```\n")

    print(f"Graph successfully generated at {out_path} with {len(nodes)} connected nodes and {len(edges)} edges.")

if __name__ == "__main__":
    generate_graph()
