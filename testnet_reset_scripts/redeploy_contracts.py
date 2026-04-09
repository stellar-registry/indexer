#!/usr/bin/env python3
import urllib.request
import json
import argparse
import subprocess
import sys

def fetch_contracts(base_url_arg="https://registry-indexer.fly.dev", starting_cursor=None, source_account="alice"):
    endpoint_url = f"{base_url_arg.rstrip('/')}/v1/contracts?limit=1"
    all_contracts = []
    cursor = starting_cursor

    while True:
        url = endpoint_url
        if cursor:
            url += f"&cursor={cursor}"

        try:
            print(f"Querying {url}...")
            with urllib.request.urlopen(url) as response:
                if response.status == 200:
                    data = json.load(response)

                    items = data.get("result", [])
                    if not items:
                        break

                    all_contracts.extend(items)
                    print(f"  Fetched {len(items)} contracts (Total: {len(all_contracts)})")

                    for c in items:
                        if c.get("channel") != "main":
                            continue
                        contract_name = c.get("contract_name")
                        deploy_url = f"{base_url_arg.rstrip('/')}/v1/contract_deploy_details/main/{contract_name}"
                        print(f"  Querying {deploy_url}...")
                        try:
                            with urllib.request.urlopen(deploy_url) as deploy_response:
                                if deploy_response.status == 200:
                                    deploy_data = json.load(deploy_response)
                                    invoke_host_function = deploy_data.get("operation_body", {}).get("invoke_host_function")
                                    invoke_host_function_json = json.dumps(invoke_host_function)
                                    print(f"    - {invoke_host_function_json}")
                                    cmd = ["stellar", "xdr", "encode", "--type", "InvokeHostFunctionOp", invoke_host_function_json]
                                    result = subprocess.run(cmd, capture_output=True, text=True)
                                    if result.returncode == 0:
                                        xdr = result.stdout.strip()
                                        print(f"    XDR: {xdr}")
                                        pipeline = (
                                            f"stellar tx new invoke --source-account {source_account} --xdr {xdr} --build-only"
                                            f" | stellar tx sign --sign-with-key {source_account}"
                                            f" | stellar tx simulate"
                                            f" | stellar tx sign --sign-with-key {source_account}"
                                            f" | stellar tx send"
                                        )
                                        tx_result = subprocess.run(pipeline, shell=True, capture_output=True, text=True)
                                        if tx_result.returncode == 0:
                                            print(f"    TX sent: {tx_result.stdout.strip()}")
                                        else:
                                            print(f"    Error sending TX: {tx_result.stderr.strip()}")
                                    else:
                                        print(f"    Error encoding XDR: {result.stderr.strip()}")
                                else:
                                    print(f"    Error fetching deploy details: HTTP status {deploy_response.status}")
                        except urllib.error.URLError as e:
                            print(f"    Error fetching deploy details: {e}")
                        except json.JSONDecodeError as e:
                            print(f"    Error parsing JSON: {e}")

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

def main():
    parser = argparse.ArgumentParser(
        description=(
            "This script redeploy all contracts from main registry, deployed before through the registry. "
            "It uses stellar cli to duplicate deploy using invoke_host_function XDR from the previous deployment"
        )
    )

    parser.add_argument("--base-url", help="Base URL for the registry indexer", default="https://registry-indexer.fly.dev")
    parser.add_argument("--cursor", help="Starting cursor for pagination", type=str)
    parser.add_argument("--source-account", help="Source account for signing transactions", required=True)

    args = parser.parse_args()

    fetch_contracts(base_url_arg=args.base_url, starting_cursor=args.cursor, source_account=args.source_account)

if __name__ == "__main__":
    main()
