#!/usr/bin/env python3
import urllib.request
import json
import sys
import argparse
import os
import subprocess

def fetch_wasms(target_directory, base_url_arg="https://registry-indexer.fly.dev", starting_cursor=None, dry_run=False):
    # Construct the full endpoint URL using the provided base-url and limit=1
    endpoint_url = f"{base_url_arg.rstrip('/')}/v1/wasms?limit=1"
    all_wasms = []
    cursor = starting_cursor
    
    # Ensure target directory exists if not dry run
    if not dry_run and not os.path.exists(target_directory):
        print(f"Creating target directory: {target_directory}")
        os.makedirs(target_directory)

    while True:
        url = endpoint_url
        if cursor:
            # Append cursor with & as we already have ?limit=1
            url += f"&cursor={cursor}"
        
        try:
            print(f"Querying {url}...")
            with urllib.request.urlopen(url) as response:
                if response.status == 200:
                    data = json.load(response)
                    
                    # Accumulate results
                    items = data.get("result", [])
                    # Break the loop if the array is empty (0 elements)
                    if not items:
                        break

                    all_wasms.extend(items)
                    print(f"  Fetched {len(items)} wasms (Total: {len(all_wasms)})")
                    
                    # Log specific WASM details and fetch
                    for w in items:
                        name = w.get("wasm_name", "N/A")
                        # Skip if channel is unverified
                        if w.get("channel") == "unverified":
                            continue
                        hash_val = w.get("wasm_hash", "N/A")
                        version = w.get("wasm_version", "N/A")
                        print(f"    - {name}@{version}: {hash_val}")
                        
                        if hash_val != "N/A":
                            # Construct output path
                            filename = f"{name}@{version}.wasm"
                            out_path = os.path.join(target_directory, filename)
                            
                            if dry_run:
                                print(f"      [DRY-RUN] Would fetch into {out_path}")
                            else:
                                # Ensure subdirectory exists (e.g., if name is unverified/xxx)
                                os.makedirs(os.path.dirname(out_path), exist_ok=True)
                                
                                print(f"      Fetching into {out_path}...")
                                cmd = [
                                    "stellar", "contract", "fetch",
                                    "--wasm-hash", hash_val,
                                    "--out-file", out_path
                                ]
                                
                                try:
                                    subprocess.run(cmd, check=True)
                                except subprocess.CalledProcessError as e:
                                    print(f"      Error fetching WASM: {e}")
                    
                    # Check for next page
                    cursor = data.get("next")
                    if not cursor:
                        break
                else:
                    print(f"Error fetching data: HTTP status {response.status}")
                    sys.exit(1)
            
        except urllib.error.URLError as e:
            print(f"Error fetching data: {e}")
            sys.exit(1)
        except json.JSONDecodeError as e:
            print(f"Error parsing JSON: {e}")
            sys.exit(1)

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Query wasms from registry-indexer with pagination.")
    parser.add_argument("--target-directory", help="Directory where files will be stored", required=True)
    parser.add_argument("--base-url", help="Base URL for the registry indexer", default="https://registry-indexer.fly.dev")
    parser.add_argument("--cursor", help="Starting cursor for pagination", type=str)
    parser.add_argument("--dry-run", help="Show the intent without fetching files", action="store_true")
    args = parser.parse_args()
    
    fetch_wasms(args.target_directory, base_url_arg=args.base_url, starting_cursor=args.cursor, dry_run=args.dry_run)
