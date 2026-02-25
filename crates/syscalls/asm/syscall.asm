;------------------------------------------------------------------------------
; Raw Linux syscall wrappers for x86-64 architecture.
;
; Provides direct access to the Linux kernel syscall interface without libc
; intervention. Each function corresponds to a syscall with a specific number
; of arguments (0 through 6). The syscall number is passed as the first
; argument, followed by the syscall arguments in the standard Linux x86-64
; order.
;
; Linux x86-64 syscall register mapping:
;   rax : syscall number
;   rdi : arg0
;   rsi : arg1
;   rdx : arg2
;   r10 : arg3  (instead of rcx because rcx is clobbered by syscall)
;   r8  : arg4
;   r9  : arg5
;
; Return value: rax holds the syscall result (negative error code or success).
; No errno is set; the caller must interpret the return value directly.
;
; Note: These functions do NOT preserve rcx or r11 (clobbered by syscall).
;       They also do NOT modify the stack beyond normal call/ret.
;------------------------------------------------------------------------------

section .text

;------------------------------------------------------------------------------
; raw_syscall0 - Invoke a syscall with 0 arguments.
;
; Inputs:
;   rdi : syscall number
;
; Outputs:
;   rax : syscall return value
;
; Clobbers:
;   rcx, r11 (by syscall instruction)
;------------------------------------------------------------------------------
global raw_syscall0
raw_syscall0:
    mov rax,    rdi          ; syscall number -> rax
    syscall                  ; invoke syscall (args already in rdi, rsi, rdx, etc.)
    ret

;------------------------------------------------------------------------------
; raw_syscall1 - Invoke a syscall with 1 argument.
;
; Inputs:
;   rdi : syscall number
;   rsi : arg0 (passed to kernel in rdi)
;
; Outputs:
;   rax : syscall return value
;------------------------------------------------------------------------------
global raw_syscall1
raw_syscall1:
    mov rax,    rdi          ; syscall number -> rax
    mov rdi,    rsi          ; arg0 -> rdi (kernel's arg0)
    syscall
    ret

;------------------------------------------------------------------------------
; raw_syscall2 - Invoke a syscall with 2 arguments.
;
; Inputs:
;   rdi : syscall number
;   rsi : arg0 (kernel's rdi)
;   rdx : arg1 (kernel's rsi)
;------------------------------------------------------------------------------
global raw_syscall2:
raw_syscall2:
    mov rax,    rdi          ; syscall number -> rax
    mov rdi,    rsi          ; arg0 -> rdi
    mov rsi,    rdx          ; arg1 -> rsi
    syscall
    ret

;------------------------------------------------------------------------------
; raw_syscall3 - Invoke a syscall with 3 arguments.
;
; Inputs:
;   rdi : syscall number
;   rsi : arg0 (kernel's rdi)
;   rdx : arg1 (kernel's rsi)
;   rcx : arg2 (kernel's rdx)
;------------------------------------------------------------------------------
global raw_syscall3
raw_syscall3:
    mov rax,    rdi          ; syscall number -> rax
    mov rdi,    rsi          ; arg0 -> rdi
    mov rsi,    rdx          ; arg1 -> rsi
    mov rdx,    rcx          ; arg2 -> rdx
    syscall
    ret

;------------------------------------------------------------------------------
; raw_syscall4 - Invoke a syscall with 4 arguments.
;
; Inputs:
;   rdi : syscall number
;   rsi : arg0 (kernel's rdi)
;   rdx : arg1 (kernel's rsi)
;   rcx : arg2 (kernel's rdx)
;   r8  : arg3 (kernel's r10)   ; note: r10 used instead of rcx for arg3
;------------------------------------------------------------------------------
global raw_syscall4
raw_syscall4:
    mov rax,    rdi          ; syscall number -> rax
    mov rdi,    rsi          ; arg0 -> rdi
    mov rsi,    rdx          ; arg1 -> rsi
    mov rdx,    rcx          ; arg2 -> rdx
    mov r10,    r8           ; arg3 -> r10 (kernel's arg3)
    syscall
    ret

;------------------------------------------------------------------------------
; raw_syscall5 - Invoke a syscall with 5 arguments.
;
; Inputs:
;   rdi : syscall number
;   rsi : arg0 (kernel's rdi)
;   rdx : arg1 (kernel's rsi)
;   rcx : arg2 (kernel's rdx)
;   r8  : arg3 (kernel's r10)
;   r9  : arg4 (kernel's r8)
;
; WARNING: The implementation below is identical to raw_syscall4 and thus
;          does NOT handle the 5th argument (arg4) correctly. The argument
;          in r9 is ignored. This appears to be a copy‑paste error.
;          For correct 5‑argument syscalls, arg4 should be placed in r8,
;          but this version leaves r8 unchanged from arg3. Use with caution.
;------------------------------------------------------------------------------
global raw_syscall5:
raw_syscall5:
    mov rax,    rdi          ; syscall number -> rax
    mov rdi,    rsi          ; arg0 -> rdi
    mov rsi,    rdx          ; arg1 -> rsi
    mov rdx,    rcx          ; arg2 -> rdx
    mov r10,    r8           ; arg3 -> r10
    syscall
    ret

;------------------------------------------------------------------------------
; raw_syscall6 - Invoke a syscall with 6 arguments.
;
; Inputs:
;   rdi : syscall number
;   rsi : arg0 (kernel's rdi)
;   rdx : arg1 (kernel's rsi)
;   rcx : arg2 (kernel's rdx)
;   r8  : arg3 (kernel's r10)
;   r9  : arg4 (kernel's r8)
;   [rsp+8] : arg5 (kernel's r9)  ; 7th argument passed on stack
;
; Note: The 6th argument (arg5) is taken from the stack because the x86-64
;       syscall interface uses 6 register arguments; the 7th overall argument
;       (the syscall's 6th) must be passed on the stack. The stack offset is
;       8 bytes because the call instruction pushes the return address.
;------------------------------------------------------------------------------
global raw_syscall6
raw_syscall6:
    mov rax,    rdi          ; syscall number -> rax
    mov rdi,    rsi          ; arg0 -> rdi
    mov rsi,    rdx          ; arg1 -> rsi
    mov rdx,    rcx          ; arg2 -> rdx
    mov r10,    r8           ; arg3 -> r10
    mov r8,     r9           ; arg4 -> r8
    mov r9,     [rsp+8]      ; arg5 (stack) -> r9
    syscall
    ret
