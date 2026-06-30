import json
import os
import re
import sys
from collections import defaultdict

def main():
    try:
        with open("errors_after_sdlc.json") as f:
            data = json.load(f)
    except FileNotFoundError:
        print("Run mev --json validate-brain --links > errors_after_sdlc.json first.")
        sys.exit(1)

    root = data["root"]
    file_errors = defaultdict(list)

    for d in data.get("diagnostics", []):
        if d["locator"].startswith("E_LINK"):
            file_errors[d["file"]].append(d)

    patched_files = 0

    for filepath_rel, errors in file_errors.items():
        filepath = os.path.join(root, filepath_rel)
        if not os.path.exists(filepath):
            continue
            
        with open(filepath, "r") as f:
            content = f.read()
            
        original_content = content
            
        for d in errors:
            msg = d["message"]
            
            if d["locator"] in ["E_LINK_DEAD_MARKDOWN", "E_LINK_DEAD_FILE_URI"]:
                m = re.search(r": '(.+?)' does not exist", msg)
                if m:
                    raw = m.group(1)
                    # 1. Try to replace [Text](raw) with Text
                    pattern1 = r'\[([^\]]+)\]\(' + re.escape(raw) + r'\)'
                    new_content = re.sub(pattern1, r'\1', content)
                    
                    if new_content == content and d["locator"] == "E_LINK_DEAD_FILE_URI":
                        # 2. Try to replace bare file:// uri with just the path
                        path_str = raw.replace("file://", "")
                        new_content = content.replace(raw, path_str)
                        
                    content = new_content
                    
            elif d["locator"] == "E_LINK_DANGLING_WIKILINK":
                m = re.search(r"dangling wikilink: '\[\[(.+?)\]\]'", msg)
                if m:
                    raw = m.group(1)
                    # Replace [[slug]] with slug
                    pattern = r'\[\[' + re.escape(raw) + r'\]\]'
                    content = re.sub(pattern, raw, content)

        if content != original_content:
            with open(filepath, "w") as f:
                f.write(content)
            print(f"Stripped dead links in {filepath_rel}")
            patched_files += 1

    print(f"Patched {patched_files} files.")

if __name__ == "__main__":
    main()
