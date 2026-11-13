#!/usr/bin/env python3
"""Compile the actual pinned range handler to verify global scheduling batches."""
import argparse
import hashlib
import pathlib
import subprocess
import tempfile

SOURCE_SHA256 = '7998a8db07c87236151607496b6d6173a0e58d8be07b74c4f241e3fd99dbf2e9'
FIXTURE = r'''
#include <algorithm>
#include <cassert>
#include <cstdint>
#include <iostream>
#include <stdexcept>
#include <string>
#include <utility>
#include <variant>
#include <vector>
namespace torrent {
using input_error=std::runtime_error;
struct Object {
 using list_type=std::vector<Object>; using list_const_iterator=list_type::const_iterator;
 std::variant<int64_t,std::string,list_type> data;
 Object():data(int64_t(0)){} Object(int64_t v):data(v){} Object(const char* v):data(std::string(v)){}
 static Object create_list(){Object o;o.data=list_type{};return o;}
 const std::string& as_string()const{return std::get<std::string>(data);}
 int64_t as_value()const{return std::get<int64_t>(data);}
 list_type& as_list(){return std::get<list_type>(data);}
}; }
struct Download {int id;bool active=false;};
std::vector<Download> downloads;
static int scheduler_calls=0, per_target_calls=0;
namespace core {
struct View {std::string name;std::vector<Download*> rows;auto begin_visible(){return rows.begin();}auto end_visible(){return rows.end();}};
struct ViewManager {
 std::vector<View*> views;using iterator=std::vector<View*>::iterator;
 iterator find(const std::string& name){return std::find_if(views.begin(),views.end(),[&](View* v){return v->name==name;});}
 iterator end(){return views.end();}
}; }
struct Control {core::ViewManager manager;core::ViewManager* view_manager(){return &manager;}};
Control controller;Control* control=&controller;
namespace rpc {
Download* make_target(Download* d){return d;}
std::pair<torrent::Object,const char*> parse_command(Download* d,const char* start,const char* end){
 std::string command(start,end);
 if(command=="scheduler.simple.update="){++scheduler_calls;for(auto& value:downloads)value.active=true;return {torrent::Object(int64_t(0)),end};}
 if(command=="d.state.set=0"){++per_target_calls;d->active=false;return {torrent::Object(int64_t(0)),end};}
 if(command=="d.hash="){++per_target_calls;return {torrent::Object(int64_t(d->id)),end};}
 throw std::runtime_error("unexpected fixture command");
} }
ACTUAL_HANDLER
int main(int argc,char**){
 bool old=argc>1;downloads.resize(1037);core::View started{"started",{}},other{"other",{}};
 for(int i=0;i<1037;++i){downloads[i].id=i;started.rows.push_back(&downloads[i]);other.rows.push_back(&downloads[i]);}
 controller.manager.views={&started,&other};
 auto call=[](const char* view,int64_t offset,int64_t limit,std::vector<torrent::Object> commands){
  torrent::Object::list_type args={torrent::Object(view),torrent::Object(offset),torrent::Object(limit)};
  args.insert(args.end(),commands.begin(),commands.end());return d_multicall_range(args);
 };
 auto first=call("started",0,1000,{torrent::Object("scheduler.simple.update=")});
 assert(first.as_list().size()==1000);assert(scheduler_calls==(old?1000:1));
 for(auto& row:first.as_list()){assert(row.as_list().size()==1);assert(row.as_list()[0].as_value()==0);}
 assert(std::all_of(downloads.begin(),downloads.end(),[](auto& d){return d.active;}));
 auto last=call("started",1000,1000,{torrent::Object("scheduler.simple.update=")});
 assert(last.as_list().size()==37);assert(scheduler_calls==(old?1037:2));
 auto empty=call("started",2000,1000,{torrent::Object("scheduler.simple.update=")});assert(empty.as_list().empty());
 int before=scheduler_calls;call("other",0,1000,{torrent::Object("scheduler.simple.update=")});assert(scheduler_calls-before==1000);
 before=scheduler_calls;call("started",0,1000,{torrent::Object("scheduler.simple.update="),torrent::Object("d.hash=")});assert(scheduler_calls-before==1000);
 per_target_calls=0;call("started",0,1000,{torrent::Object("d.state.set=0")});assert(per_target_calls==1000);
 assert(std::count_if(downloads.begin(),downloads.end(),[](auto& d){return !d.active;})==1000);
 std::cout<<"actual range handler: 1037 started rows; global passes="<<(old?1037:2)<<"; row shape, pagination, mixed commands and per-target mutation preserved\n";
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=pathlib.Path, required=True)
    parser.add_argument('--patch', type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[2] / 'deploy/docker/patches/rtorrent-0.16.28-multicall-range.patch')
    parser.add_argument('--expect-original-repetition', action='store_true')
    args = parser.parse_args()
    raw = args.source.read_bytes()
    assert hashlib.sha256(raw).hexdigest() == SOURCE_SHA256
    with tempfile.TemporaryDirectory(prefix='torrent-scheduler-regression-') as scratch:
        root = pathlib.Path(scratch);vendor=root/'src/command_events.cc';vendor.parent.mkdir();vendor.write_bytes(raw)
        subprocess.run(['patch','--batch','--fuzz=0','-p1','-i',str(args.patch.resolve())],cwd=root,check=True)
        source=vendor.read_text();start=source.index('static constexpr int64_t tng_multicall_max_limit');end=source.index('\ntorrent::Object\nd_multicall_nonzero_rate',start)
        fixture=root/'fixture.cc';fixture.write_text(FIXTURE.replace('ACTUAL_HANDLER',source[start:end]))
        binary=root/'fixture';subprocess.run(['g++','-std=c++17','-O2','-Wall','-Wextra','-Werror',str(fixture),'-o',str(binary)],check=True)
        subprocess.run([str(binary),*(['original'] if args.expect_original_repetition else [])],check=True)


if __name__ == '__main__':
    main()
