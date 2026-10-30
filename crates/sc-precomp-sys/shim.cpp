// Unity build: precomp.cpp is one 8500-line translation unit with global state, so the wrapper
// is compiled into it. Precomp calls exit() on errors, keeps its state in globals and writes
// ~temp*.dat into the working directory; the Rust side therefore only ever calls this inside a
// short-lived child process whose working directory is private.
#include "precomp.cpp"

extern "C" {

// mode 0: precompress `in_file` into `out_file` (.pcf, stored uncompressed so that another
// codec can compress it); mode 1: restore. Returns 0 on success, 1 on failure; `msg` (256 bytes)
// receives the failure text.
int sc_precomp_run(int mode, const char* in_file, const char* out_file, char* msg) {
    Switches sw;
    sw.compression_method = OTF_NONE;
    // Level search stays at the default (all zlib levels); intense/brute modes are off for speed.
    std::string in(in_file), out(out_file);
    msg[0] = 0;
    bool ok = (mode == 0) ? precompress_file(&in[0], &out[0], msg, sw) : recompress_file(&in[0], &out[0], msg, sw);
    fflush(stdout);
    if (fin != NULL) fclose(fin);
    if (fout != NULL) fclose(fout);
    return ok ? 0 : 1;
}

}  // extern "C"
