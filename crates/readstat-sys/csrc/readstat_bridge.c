#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include "readstat.h"

typedef int (*rs_metadata_cb)(
    int64_t row_count,
    int64_t var_count,
    const char *table_name,
    const char *file_label,
    const char *file_encoding,
    int is64bit,
    int compression,
    void *ctx
);

typedef int (*rs_variable_cb)(
    int index,
    const char *name,
    const char *label,
    const char *format,
    int is_string,
    size_t storage_width,
    int display_width,
    void *ctx
);

typedef int (*rs_value_cb)(
    int obs_index,
    int var_index,
    int is_string,
    int missing_kind, /* 0 = valid, 1 = system missing, 2 = tagged missing */
    char tag,
    double num_val,
    const char *str_val,
    void *ctx
);

typedef int (*rs_progress_cb)(double progress, void *ctx);

typedef struct rs_bridge_ctx_s {
    rs_metadata_cb on_metadata;
    rs_variable_cb on_variable;
    rs_value_cb    on_value;
    rs_progress_cb on_progress;
    void          *user_ctx;
} rs_bridge_ctx_t;

static int bridge_metadata_handler(readstat_metadata_t *metadata, void *ctx) {
    rs_bridge_ctx_t *b = (rs_bridge_ctx_t *)ctx;
    if (!b || !b->on_metadata) return READSTAT_HANDLER_OK;
    return b->on_metadata(
        (int64_t)readstat_get_row_count(metadata),
        (int64_t)readstat_get_var_count(metadata),
        readstat_get_table_name(metadata),
        readstat_get_file_label(metadata),
        readstat_get_file_encoding(metadata),
        readstat_get_file_format_is_64bit(metadata),
        (int)readstat_get_compression(metadata),
        b->user_ctx
    );
}

static int bridge_variable_handler(int index, readstat_variable_t *variable,
        const char *val_labels, void *ctx) {
    (void)val_labels;
    rs_bridge_ctx_t *b = (rs_bridge_ctx_t *)ctx;
    if (!b || !b->on_variable) return READSTAT_HANDLER_OK;
    int is_string = (readstat_variable_get_type_class(variable) == READSTAT_TYPE_CLASS_STRING) ? 1 : 0;
    return b->on_variable(
        index,
        readstat_variable_get_name(variable),
        readstat_variable_get_label(variable),
        readstat_variable_get_format(variable),
        is_string,
        readstat_variable_get_storage_width(variable),
        readstat_variable_get_display_width(variable),
        b->user_ctx
    );
}

static int bridge_value_handler(int obs_index, readstat_variable_t *variable,
        readstat_value_t value, void *ctx) {
    rs_bridge_ctx_t *b = (rs_bridge_ctx_t *)ctx;
    if (!b || !b->on_value) return READSTAT_HANDLER_OK;

    int var_index = readstat_variable_get_index(variable);
    int is_string = (readstat_value_type_class(value) == READSTAT_TYPE_CLASS_STRING) ? 1 : 0;
    int missing_kind = 0;
    char tag = 0;
    double num_val = 0.0;
    const char *str_val = NULL;

    if (readstat_value_is_system_missing(value)) {
        missing_kind = 1;
    } else if (readstat_value_is_tagged_missing(value)) {
        missing_kind = 2;
        tag = readstat_value_tag(value);
    } else if (is_string) {
        str_val = readstat_string_value(value);
    } else {
        num_val = readstat_double_value(value);
    }

    return b->on_value(
        obs_index,
        var_index,
        is_string,
        missing_kind,
        tag,
        num_val,
        str_val,
        b->user_ctx
    );
}

static int bridge_progress_handler(double progress, void *ctx) {
    rs_bridge_ctx_t *b = (rs_bridge_ctx_t *)ctx;
    if (!b || !b->on_progress) return READSTAT_HANDLER_OK;
    return b->on_progress(progress, b->user_ctx);
}

int rs_bridge_parse_file(
    const char *path,
    int format_kind, /* 0 = sas7bdat, 1 = xport */
    const char *input_encoding, /* NULL for auto */
    long row_offset,
    long row_limit, /* 0 for unlimited */
    rs_metadata_cb on_metadata,
    rs_variable_cb on_variable,
    rs_value_cb on_value,
    rs_progress_cb on_progress,
    void *user_ctx,
    char *err_buf,
    size_t err_buf_len
) {
    readstat_parser_t *parser = readstat_parser_init();
    if (!parser) {
        if (err_buf && err_buf_len > 0) {
            snprintf(err_buf, err_buf_len, "Failed to initialize ReadStat parser");
        }
        return (int)READSTAT_ERROR_MALLOC;
    }

    rs_bridge_ctx_t bctx;
    bctx.on_metadata = on_metadata;
    bctx.on_variable = on_variable;
    bctx.on_value = on_value;
    bctx.on_progress = on_progress;
    bctx.user_ctx = user_ctx;

    if (on_metadata) readstat_set_metadata_handler(parser, bridge_metadata_handler);
    if (on_variable) readstat_set_variable_handler(parser, bridge_variable_handler);
    if (on_value)    readstat_set_value_handler(parser, bridge_value_handler);
    if (on_progress) readstat_set_progress_handler(parser, bridge_progress_handler);

    if (input_encoding && input_encoding[0] != '\0') {
        readstat_set_file_character_encoding(parser, input_encoding);
    }
    readstat_set_handler_character_encoding(parser, "UTF-8");

    if (row_offset > 0) {
        readstat_set_row_offset(parser, row_offset);
    }
    if (row_limit > 0) {
        readstat_set_row_limit(parser, row_limit);
    }

    readstat_error_t err;
    if (format_kind == 1) {
        err = readstat_parse_xport(parser, path, &bctx);
    } else {
        err = readstat_parse_sas7bdat(parser, path, &bctx);
    }

    if (err != READSTAT_OK && err_buf && err_buf_len > 0) {
        const char *msg = readstat_error_message(err);
        snprintf(err_buf, err_buf_len, "%s", msg ? msg : "Unknown ReadStat error");
    }

    readstat_parser_free(parser);
    return (int)err;
}

/* ===== Writer Bridge (for tests & sample dataset generation) ===== */

static ssize_t bridge_file_writer(const void *data, size_t len, void *ctx) {
    FILE *fp = (FILE *)ctx;
    if (!fp) return -1;
    size_t written = fwrite(data, 1, len, fp);
    return (written == len) ? (ssize_t)written : -1;
}

typedef struct rs_writer_col_s {
    const char *name;
    const char *label;
    const char *format;
    int         is_string;
    size_t      storage_width;
} rs_writer_col_t;

typedef struct rs_writer_handle_s {
    readstat_writer_t    *writer;
    FILE                 *fp;
    readstat_variable_t **vars;
    int                   var_count;
} rs_writer_handle_t;

rs_writer_handle_t *rs_bridge_writer_open(
    const char *path,
    int format_kind, /* 0 = sas7bdat, 1 = xport */
    const char *table_name,
    const char *file_label,
    const rs_writer_col_t *cols,
    int col_count,
    long row_count,
    int compress_rle
) {
    FILE *fp = fopen(path, "wb");
    if (!fp) return NULL;

    readstat_writer_t *w = readstat_writer_init();
    if (!w) {
        fclose(fp);
        return NULL;
    }

    readstat_set_data_writer(w, bridge_file_writer);
    if (table_name && table_name[0]) readstat_writer_set_table_name(w, table_name);
    if (file_label && file_label[0]) readstat_writer_set_file_label(w, file_label);
    if (format_kind == 0) {
        readstat_writer_set_file_format_is_64bit(w, 1);
        if (compress_rle) {
            readstat_writer_set_compression(w, READSTAT_COMPRESS_ROWS);
        }
    } else {
        readstat_writer_set_file_format_version(w, 5);
    }

    readstat_variable_t **vars = calloc((size_t)col_count, sizeof(readstat_variable_t *));
    for (int i = 0; i < col_count; i++) {
        readstat_type_t rtype = cols[i].is_string ? READSTAT_TYPE_STRING : READSTAT_TYPE_DOUBLE;
        size_t width = cols[i].is_string ? cols[i].storage_width : 8;
        readstat_variable_t *v = readstat_add_variable(w, cols[i].name, rtype, width);
        if (!v) {
            free(vars);
            readstat_writer_free(w);
            fclose(fp);
            return NULL;
        }
        if (cols[i].label && cols[i].label[0]) {
            readstat_variable_set_label(v, cols[i].label);
        }
        if (cols[i].format && cols[i].format[0]) {
            readstat_variable_set_format(v, cols[i].format);
        }
        vars[i] = v;
    }

    readstat_error_t err;
    if (format_kind == 1) {
        err = readstat_begin_writing_xport(w, fp, row_count);
    } else {
        err = readstat_begin_writing_sas7bdat(w, fp, row_count);
    }
    if (err != READSTAT_OK) {
        free(vars);
        readstat_writer_free(w);
        fclose(fp);
        return NULL;
    }

    rs_writer_handle_t *h = calloc(1, sizeof(rs_writer_handle_t));
    h->writer = w;
    h->fp = fp;
    h->vars = vars;
    h->var_count = col_count;
    return h;
}

int rs_bridge_writer_begin_row(rs_writer_handle_t *h) {
    return (int)readstat_begin_row(h->writer);
}

int rs_bridge_writer_put_double(rs_writer_handle_t *h, int col_idx, double val) {
    return (int)readstat_insert_double_value(h->writer, h->vars[col_idx], val);
}

int rs_bridge_writer_put_string(rs_writer_handle_t *h, int col_idx, const char *val) {
    return (int)readstat_insert_string_value(h->writer, h->vars[col_idx], val);
}

int rs_bridge_writer_put_missing(rs_writer_handle_t *h, int col_idx) {
    return (int)readstat_insert_missing_value(h->writer, h->vars[col_idx]);
}

int rs_bridge_writer_put_tagged_missing(rs_writer_handle_t *h, int col_idx, char tag) {
    return (int)readstat_insert_tagged_missing_value(h->writer, h->vars[col_idx], tag);
}

int rs_bridge_writer_end_row(rs_writer_handle_t *h) {
    return (int)readstat_end_row(h->writer);
}

int rs_bridge_writer_close(rs_writer_handle_t *h) {
    if (!h) return 0;
    readstat_error_t err = readstat_end_writing(h->writer);
    readstat_writer_free(h->writer);
    fclose(h->fp);
    free(h->vars);
    free(h);
    return (int)err;
}
