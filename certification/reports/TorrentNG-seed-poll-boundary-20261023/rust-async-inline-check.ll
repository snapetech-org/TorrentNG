; ModuleID = 'async_inline_check.fc3c283a1de857ca-cgu.0'
source_filename = "async_inline_check.fc3c283a1de857ca-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

; Function Attrs: nounwind nonlazybind optsize uwtable
define { i32, i32 } @caller() unnamed_addr #0 personality ptr @rust_eh_personality {
bb6:
  %0 = alloca [4 x i8], align 4
  call void @llvm.lifetime.start.p0(ptr nonnull %0)
  store i32 123, ptr %0, align 4
  call void asm sideeffect "", "r,~{memory}"(ptr nonnull %0) #2, !srcloc !3
  %_0.i.i = load i32, ptr %0, align 4, !noundef !4
  call void @llvm.lifetime.end.p0(ptr nonnull %0)
  %1 = insertvalue { i32, i32 } { i32 0, i32 poison }, i32 %_0.i.i, 1
  ret { i32, i32 } %1
}

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.lifetime.start.p0(ptr captures(none)) #1

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.lifetime.end.p0(ptr captures(none)) #1

; Function Attrs: nounwind nonlazybind optsize uwtable
declare noundef range(i32 0, 10) i32 @rust_eh_personality(i32 noundef, i32 noundef, i64 noundef, ptr noundef, ptr noundef) unnamed_addr #0

attributes #0 = { nounwind nonlazybind optsize uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #1 = { mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite) }
attributes #2 = { nounwind }

!llvm.module.flags = !{!0, !1}
!llvm.ident = !{!2}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{!"rustc version 1.97.0 (2d8144b78 2026-07-07)"}
!3 = !{i64 1766163566970414}
!4 = !{}
