#!/usr/bin/env python3
"""Compile the actual patched event_read against a deterministic socket fixture."""
import argparse
import hashlib
import pathlib
import subprocess
import tempfile

SOURCE_SHA256 = 'b1bc696a143501007fd7a725ec493f8269de55a920a7226425f311ff1baadd22'
FIXTURE = r'''
#include <algorithm>
#include <cerrno>
#include <cstring>
#include <functional>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>
static int pending, discarded, opened;
namespace torrent {
using internal_error = std::runtime_error;
using resource_error = std::runtime_error;
int fd_accept(int) { if (pending) { --pending; return 100 + pending; } errno=EAGAIN; return -1; }
void fd_close(int) { ++discarded; }
namespace runtime {
struct Manager {
  template<class Task> bool open_event_or_cleanup(Task* task, std::function<void()> open, std::function<void()> cleanup) {
    if (task->is_open()) throw internal_error("SocketManager::open_event_or_cleanup(): event is already open");
    open();
    if (!task->is_open()) { cleanup(); return false; }
    return true;
  }
};
Manager* socket_manager() { static Manager manager; return &manager; }
}}
class SCgi {
public:
  struct Task {
    bool active=true;
    bool is_open() const { return active; }
    void open(SCgi*, int) { active=true; ++opened; }
    void cancel_open() { active=false; }
  };
  std::vector<std::unique_ptr<Task>> m_tasks;
  decltype(m_tasks)::iterator m_current;
  SCgi(int cursor, int free_slot) {
    for (int i=0;i<8;++i) { auto task=std::make_unique<Task>(); task->active=(i!=free_slot); m_tasks.push_back(std::move(task)); }
    m_current=m_tasks.begin()+cursor;
  }
  int file_descriptor() const { return 42; }
  void event_read();
};
ACTUAL_EVENT_READ
int main(int argc, char**) {
  const bool expect_defect=argc>1;
  int cases=0, defects=0;
  for (int cursor=0;cursor<8;++cursor) for (int free_slot=-1;free_slot<8;++free_slot) {
    SCgi scgi(cursor,free_slot); pending=4;discarded=0;opened=0;++cases;
    try {
      scgi.event_read();
      if (pending!=0 || opened!=(free_slot<0?0:1) || discarded!=4-opened) throw std::runtime_error("socket accounting mismatch");
    } catch (const std::runtime_error& error) {
      if (!expect_defect || std::string(error.what()).find("event is already open")==std::string::npos) throw;
      ++defects;
    }
  }
  if (expect_defect && defects==0) throw std::runtime_error("original defect was not reproduced");
  std::cout << "actual event_read: cases=" << cases << " reproduced_defects=" << defects << "\n";
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=pathlib.Path, required=True,
                        help='Unmodified src/rpc/scgi.cc from pinned rTorrent commit 2700b3141f966d8533a6153491e1c76b3cf5c2b9')
    parser.add_argument('--patch', type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[2] / 'deploy/docker/patches/rtorrent-0.16.11-scgi-accept-eagain.patch')
    parser.add_argument('--expect-original-defect', action='store_true')
    args = parser.parse_args()
    source = args.source.read_bytes()
    assert hashlib.sha256(source).hexdigest() == SOURCE_SHA256, 'pinned upstream source changed'
    with tempfile.TemporaryDirectory(prefix='torrent-scgi-regression-') as scratch:
        root = pathlib.Path(scratch)
        vendor = root / 'src/rpc/scgi.cc'
        vendor.parent.mkdir(parents=True)
        vendor.write_bytes(source)
        subprocess.run(['patch', '--batch', '--fuzz=0', '-p1', '-i', str(args.patch.resolve())], cwd=root, check=True)
        patched = vendor.read_text()
        start = patched.index('void\nSCgi::event_read() {')
        end = patched.index('\nvoid\nSCgi::event_write()', start)
        fixture = root / 'regression.cc'
        fixture.write_text(FIXTURE.replace('ACTUAL_EVENT_READ', patched[start:end]))
        binary = root / 'regression'
        subprocess.run(['g++', '-std=c++17', '-O2', '-Wall', '-Wextra', '-Werror', str(fixture), '-o', str(binary)], check=True)
        subprocess.run([str(binary), *(['expect-defect'] if args.expect_original_defect else [])], check=True)


if __name__ == '__main__':
    main()
