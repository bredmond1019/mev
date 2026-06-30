import json
import os
import sys

def main():
    try:
        with open("errors_after_sdlc.json") as f:
            data = json.load(f)
    except FileNotFoundError:
        print("Run mev --json validate-brain --links > errors_after_sdlc.json first.")
        sys.exit(1)

    root = data["root"]
    patched = 0
    
    for d in data.get("diagnostics", []):
        if d["locator"] == "frontmatter":
            filepath = os.path.join(root, d["file"])
            if not os.path.exists(filepath):
                continue
                
            with open(filepath, "r") as md:
                content = md.read()
                
            if content.strip().startswith("---"):
                continue # Safety check
                
            filename = os.path.basename(filepath)
            stem = os.path.splitext(filename)[0]
            
            # Basic OKF frontmatter template
            frontmatter = f"""---
type: Reference
title: {stem.replace('-', ' ').title()}
description: Documentation for {stem}
doc_id: {stem}
layer: [meta]
project: {d["file"].split('/')[0] if '/' in d["file"] else 'root'}
status: active
---

"""
            with open(filepath, "w") as md:
                md.write(frontmatter + content)
            print(f"Added OKF frontmatter to {d['file']}")
            patched += 1

    print(f"Patched {patched} files.")

if __name__ == "__main__":
    main()
