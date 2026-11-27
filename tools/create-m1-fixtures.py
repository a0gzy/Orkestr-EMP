"""Explicit, synthetic M1 smoke inputs. Never reads real application data or executes commands."""
import argparse
import datetime as dt
import json
from pathlib import Path
import sqlite3
import uuid


def write_json(path, data):
    with path.open("x", encoding="utf-8") as file:
        json.dump(data, file, ensure_ascii=False, indent=2)


def make(out):
    out.mkdir(parents=True, exist_ok=False)
    db = sqlite3.connect(out / "inventory-copy.sqlite")
    db.executescript("CREATE TABLE computers(host TEXT PRIMARY KEY COLLATE NOCASE,guid TEXT NOT NULL,data TEXT NOT NULL); CREATE TABLE collections(id TEXT PRIMARY KEY,data TEXT NOT NULL);")
    stamp = dt.datetime.now(dt.timezone.utc).isoformat()
    hosts = []
    for i in range(50):
        host = f"fixture-pc-{i+1:02}.example.test"
        hosts.append(host)
        ram = [4, 8, 12, 16, 32, None][i % 6]
        details = {
            "Manufacturer": "SYNTHETIC TEST ONLY", "Model": "Fixture workstation",
            "Serial": f"NOT-REAL-{i:04}", "CPU": "Synthetic CPU",
            "MemoryModules": [{"Capacity": ram * 2**30}] if ram else [],
            "Software": [{"Name": "Fixture Chrome", "Publisher": "Fixture Vendor", "Version": "2.9" if i % 2 else "2.10"}],
            "OSVersion": "Fixture, not scanned", "Build": "Synthetic", "Architecture": "64-bit fixture",
            "BIOSVersion": "Synthetic BIOS", "BIOSManufacturer": "Fixture Vendor",
            "Disks": [{"DeviceID": "C:", "Size": 512 * 2**30, "FreeSpace": 128 * 2**30}],
            "CPUs": [{"Name": "Synthetic CPU", "NumberOfCores": 4}],
            "NICs": [{"Description": "Synthetic NIC", "IPAddress": [f"192.0.2.{i+1}"]}],
            "HotFixes": [], "LocalUsers": [{"Name": "fixture-user", "Disabled": False}],
            "LocalGroups": [{"Name": "Fixture users"}], "Printers": [], "Shares": [],
            "Processes": [{"Name": "synthetic.exe", "ProcessId": 1}],
            "Services": [{"Name": "fixture-service", "State": "Synthetic"}],
            "Environment": [{"Name": "M1_FIXTURE", "VariableValue": "NOT_A_SECRET"}],
            "WindowsFeatures": [], "Startup": [], "TaskSchedules": [],
            "Warnings": ["SYNTHETIC: no scan occurred; legacy software covers HKLM only"],
        }
        row = {"host": host, "name": f"Fixture PC {i+1:02}", "domain": "example.test", "guid": str(uuid.uuid5(uuid.NAMESPACE_DNS, host)),
               "os": "Windows 11 · fixture", "ip": f"192.0.2.{i+1}", "dn": "OU=Fixtures,DC=example,DC=test", "source": "SYNTHETIC FIXTURE",
               "disabled": False, "last_scan": stamp, "status": "Synthetic, no scan", "details": json.dumps(details, ensure_ascii=False), "ad_details": "Synthetic directory data"}
        db.execute("INSERT INTO computers VALUES(?,?,?)", (host, row["guid"], json.dumps(row, ensure_ascii=False)))
    for group in [{"id": "pilot", "name": "Пилот · учебная", "dynamic": False, "hosts": hosts[:5]},
                  {"id": "old-ram-rule", "name": "Legacy RAM · для review", "dynamic": True, "rules": [{"field": "MemoryGB", "operator": "greater than", "value": "8"}]}]:
        db.execute("INSERT INTO collections VALUES(?,?)", (group["id"], json.dumps(group, ensure_ascii=False)))
    db.commit()
    db.close()
    write_json(out / "console-copy.json", {
        "packages": [{"id": 1, "name": "Учебный пакет · не исполняется", "version": "1.0", "description": "SYNTHETIC TEST ONLY — no actual command or installer runs", "folder": "Fixtures",
                      "steps": [{"kind": "PowerShell", "title": "Synthetic read-only definition", "command": "Write-Output 'fixture: never executed'", "enabled": True},
                                {"kind": "Sleep", "title": "Synthetic delay", "seconds": 1, "enabled": True}]}],
        "lists": [{"name": "Учебные цели", "computers": ";".join(hosts[:3])}],
        "schedules": [{"id": 1, "name": "Учебное расписание · OFF after import", "package_id": 1, "targets": hosts[0], "next": 1800000000, "recurrence": "weekly", "enabled": True, "last": "Fixture only"}],
        "deployments": [{"id": 1, "package_id": 1, "package_name": "Синтетическая история", "created": 1700000000, "finished": True, "retries": 0,
                         "targets": [{"computer": hosts[0], "status": "Fixture — never executed", "step": 1, "total": 1, "output": "MUST_NOT_IMPORT_OUTPUT", "elapsed": 0}]}],
    })
    group_dir = out / "groups"
    group_dir.mkdir()
    def group(index, name, kind, parent=None):
        id_ = str(uuid.uuid5(uuid.NAMESPACE_URL, f"orkestr-fixture/{index}/{name}"))
        write_json(group_dir / f"{index:02}.json", {"id": id_, "parent_id": parent, "name": name, "revision": 0, "kind": kind})
        return id_
    def ram(op, gib):
        return {"Ram": {"op": op, "bytes": gib * 2**30}}
    hardware = group(1, "Оборудование", "Folder")
    memory = group(2, "Оперативная память", "Folder", hardware)
    group(3, "16 GiB и более", {"Dynamic": ram("Ge", 16)}, memory)
    group(4, "8 GiB и более", {"Dynamic": ram("Ge", 8)}, memory)
    group(5, "От 8 до <16 GiB", {"Dynamic": {"And": [ram("Ge", 8), ram("Lt", 16)]}}, memory)
    group(6, "Менее 8 GiB", {"Dynamic": ram("Lt", 8)}, memory)
    group(7, "RAM · Unknown", {"Dynamic": "RamUnknown"}, memory)
    apps = group(8, "Приложения", "Folder")
    chrome = group(9, "Fixture Chrome", "Folder", apps)
    group(10, "Ниже 2.10 · fixture", {"Dynamic": {"Software": {"application_id": None, "name": "Fixture Chrome", "publisher": "Fixture Vendor", "channel": None, "architecture": None, "version": ["Lt", {"Literal": "2.10"}]}}}, chrome)
    print(f"Created SYNTHETIC ONLY inputs in {out}; no scan/network/execution occurred.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path, help="NEW directory, never overwritten")
    make(parser.parse_args().out)
