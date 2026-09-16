// Walk private memory regions of a process and bucket by Type/State.
// Usage: after loading, [VQ]::Dump($pid)
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

public static class VQ {
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern int VirtualQueryEx(IntPtr h, IntPtr addr, out MEMORY_BASIC_INFORMATION info, int len);

    [StructLayout(LayoutKind.Sequential)]
    struct MEMORY_BASIC_INFORMATION {
        public UIntPtr BaseAddress;
        public UIntPtr AllocationBase;
        public uint AllocationProtect;
        public UIntPtr RegionSize;
        public uint State;
        public uint Protect;
        public uint Type;
    }

    const uint PROCESS_QUERY_INFORMATION = 0x0400;
    const uint MEM_COMMIT = 0x1000;
    const uint MEM_RESERVE = 0x2000;
    const uint MEM_FREE = 0x10000;
    const uint MEM_PRIVATE = 0x20000;
    const uint MEM_MAPPED = 0x40000;
    const uint MEM_IMAGE = 0x1000000;

    static string TypeName(uint t) {
        if (t == MEM_PRIVATE) return "Private";
        if (t == MEM_MAPPED) return "Mapped";
        if (t == MEM_IMAGE) return "Image";
        return t.ToString("X");
    }
    static string StateName(uint s) {
        if (s == MEM_COMMIT) return "Commit";
        if (s == MEM_RESERVE) return "Reserve";
        if (s == MEM_FREE) return "Free";
        return s.ToString("X");
    }
    static string ProtectName(uint p) {
        // ignore PAGE_GUARD 0x100 / NOCACHE etc for bucketing
        uint basep = p & 0xFF;
        switch (basep) {
            case 0x01: return "NA";
            case 0x02: return "RO";
            case 0x04: return "RW";
            case 0x08: return "WC";
            case 0x10: return "X";
            case 0x20: return "XR";
            case 0x40: return "XW";
            case 0x80: return "XWC";
            default: return p.ToString("X");
        }
    }

    public static string Dump(int pid) {
        IntPtr h = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid);
        if (h == IntPtr.Zero) return "OpenProcess failed";
        try {
            var buckets = new Dictionary<string, ulong>();
            long addr = 0;
            long max = 0x7FFFFFFFFFFF; // user space limit x64
            var info = new MEMORY_BASIC_INFORMATION();
            int sz = Marshal.SizeOf(typeof(MEMORY_BASIC_INFORMATION));
            ulong totalCommitPrivate = 0, totalCommitImage = 0, totalCommitMapped = 0, totalReserve = 0;
            int regions = 0;
            while (addr < max) {
                int q = VirtualQueryEx(h, new IntPtr(addr), out info, sz);
                if (q == 0) break;
                ulong baseAddr = info.BaseAddress.ToUInt64();
                ulong size = info.RegionSize.ToUInt64();
                if (size == 0) break;
                if (info.State == MEM_COMMIT) {
                    string key = TypeName(info.Type) + "/" + ProtectName(info.Protect);
                    ulong prev;
                    buckets.TryGetValue(key, out prev);
                    buckets[key] = prev + size;
                    if (info.Type == MEM_PRIVATE) totalCommitPrivate += size;
                    else if (info.Type == MEM_IMAGE) totalCommitImage += size;
                    else if (info.Type == MEM_MAPPED) totalCommitMapped += size;
                    regions++;
                } else if (info.State == MEM_RESERVE && info.Type == MEM_PRIVATE) {
                    totalReserve += size;
                }
                long next = (long)(baseAddr + size);
                if (next <= addr) break;
                addr = next;
            }
            var sb = new StringBuilder();
            sb.AppendLine("pid=" + pid);
            sb.AppendLine(string.Format("CommitPrivate={0:F1}MB CommitImage={1:F1}MB CommitMapped={2:F1}MB ReservePrivate={3:F1}MB regions={4}",
                totalCommitPrivate / 1048576.0, totalCommitImage / 1048576.0,
                totalCommitMapped / 1048576.0, totalReserve / 1048576.0, regions));
            sb.AppendLine("Commit buckets (top):");
            int n = 0;
            foreach (var kv in new List<KeyValuePair<string, ulong>>(buckets)) { }
            var list = new List<KeyValuePair<string, ulong>>(buckets);
            list.Sort((a, b) => b.Value.CompareTo(a.Value));
            foreach (var kv in list) {
                sb.AppendLine(string.Format("  {0,-12} {1,8:F2} MB", kv.Key, kv.Value / 1048576.0));
                if (++n >= 20) break;
            }
            return sb.ToString();
        } finally {
            CloseHandle(h);
        }
    }
}
