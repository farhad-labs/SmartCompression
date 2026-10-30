/* Flat in-memory API over 7-Zip's LZMA2 / PPMd7 / BCJ code. All sizes are bytes;
 * every function returns 0 on success and a non-zero SRes-style code otherwise. */
#include "Precomp.h"
#include "Alloc.h"
#include "Bra.h"
#include "Lzma2Dec.h"
#include "Lzma2Enc.h"
#include "Ppmd7.h"

int sc_lzma2_encode(const Byte *src, size_t n, Byte *dst, size_t *dst_len, int level, UInt32 dict,
                    int lc, int lp, int pb, int fb, Byte *prop_out)
{
  CLzma2EncHandle h = Lzma2Enc_Create(&g_Alloc, &g_BigAlloc);
  CLzma2EncProps props;
  SRes res;
  if (!h) return SZ_ERROR_MEM;
  Lzma2EncProps_Init(&props);
  props.lzmaProps.level = level;
  props.lzmaProps.dictSize = dict;
  if (lc >= 0) props.lzmaProps.lc = lc;
  if (lp >= 0) props.lzmaProps.lp = lp;
  if (pb >= 0) props.lzmaProps.pb = pb;
  if (fb > 0) props.lzmaProps.fb = fb;
  props.lzmaProps.numThreads = 1;
  props.numTotalThreads = 1;
  props.blockSize = LZMA2_ENC_PROPS_BLOCK_SIZE_SOLID;
  res = Lzma2Enc_SetProps(h, &props);
  if (res == SZ_OK) {
    *prop_out = Lzma2Enc_WriteProperties(h);
    res = Lzma2Enc_Encode2(h, NULL, dst, dst_len, NULL, src, n, NULL);
  }
  Lzma2Enc_Destroy(h);
  return (int)res;
}

int sc_lzma2_decode(Byte *dst, size_t dst_len, const Byte *src, size_t src_len, Byte prop)
{
  SizeT out = dst_len, in = src_len;
  ELzmaStatus status;
  SRes res = Lzma2Decode(dst, &out, src, &in, prop, LZMA_FINISH_END, &status, &g_Alloc);
  if (res == SZ_OK && out != dst_len) return SZ_ERROR_DATA;
  return (int)res;
}

/* ---- PPMd7 (PPMd var.H with the 7z range coder) ---- */

typedef struct { IByteOut vt; Byte *buf; size_t pos, cap; int overflow; } OutBuf;
typedef struct { IByteIn vt; const Byte *buf; size_t pos, len; } InBuf;

static void out_write(IByteOutPtr p, Byte b)
{
  OutBuf *o = (OutBuf *)(void *)p;
  if (o->pos < o->cap) o->buf[o->pos++] = b; else o->overflow = 1;
}

static Byte in_read(IByteInPtr p)
{
  InBuf *i = (InBuf *)(void *)p;
  return i->pos < i->len ? i->buf[i->pos++] : 0;
}

int sc_ppmd7_encode(const Byte *src, size_t n, Byte *dst, size_t *dst_len, unsigned order, UInt32 mem)
{
  CPpmd7 p;
  OutBuf o;
  o.vt.Write = out_write; o.buf = dst; o.pos = 0; o.cap = *dst_len; o.overflow = 0;
  Ppmd7_Construct(&p);
  if (!Ppmd7_Alloc(&p, mem, &g_Alloc)) return SZ_ERROR_MEM;
  p.rc.enc.Stream = &o.vt;
  Ppmd7z_Init_RangeEnc(&p);
  Ppmd7_Init(&p, order);
  Ppmd7z_EncodeSymbols(&p, src, src + n);
  Ppmd7z_Flush_RangeEnc(&p);
  Ppmd7_Free(&p, &g_Alloc);
  if (o.overflow) return SZ_ERROR_OUTPUT_EOF;
  *dst_len = o.pos;
  return SZ_OK;
}

int sc_ppmd7_decode(Byte *dst, size_t n, const Byte *src, size_t src_len, unsigned order, UInt32 mem)
{
  CPpmd7 p;
  InBuf in;
  size_t i;
  int res = SZ_OK;
  in.vt.Read = in_read; in.buf = src; in.pos = 0; in.len = src_len;
  Ppmd7_Construct(&p);
  if (!Ppmd7_Alloc(&p, mem, &g_Alloc)) return SZ_ERROR_MEM;
  p.rc.dec.Stream = &in.vt;
  if (!Ppmd7z_RangeDec_Init(&p.rc.dec)) { Ppmd7_Free(&p, &g_Alloc); return SZ_ERROR_DATA; }
  Ppmd7_Init(&p, order);
  for (i = 0; i < n; i++) {
    int sym = Ppmd7z_DecodeSymbol(&p);
    if (sym < 0) { res = SZ_ERROR_DATA; break; }
    dst[i] = (Byte)sym;
  }
  Ppmd7_Free(&p, &g_Alloc);
  return res;
}

/* ---- x86 BCJ, whole buffer, in place. The last few bytes are left unconverted in
 * both directions, so encode and decode stay symmetric. ---- */
void sc_x86_bcj(Byte *data, size_t n, int encode)
{
  UInt32 state = Z7_BRANCH_CONV_ST_X86_STATE_INIT_VAL;
  if (encode) z7_BranchConvSt_X86_Enc(data, n, 0, &state);
  else z7_BranchConvSt_X86_Dec(data, n, 0, &state);
}
