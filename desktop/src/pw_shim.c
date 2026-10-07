/* Thin C shim around libpipewire-0.3: one native PipeWire stream that IS an
 * audio source ("Vibe Phone Mic"). No modules are loaded; the node lives as
 * long as this client is connected, so it vanishes automatically if the app
 * dies. Samples are pulled from Rust in the realtime process callback.
 *
 * Built by build.rs with the cc crate; links the system libpipewire-0.3.so.0.
 */
#include <errno.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <pthread.h>

#include <pipewire/pipewire.h>
#include <spa/node/io.h>
#include <spa/param/audio/format-utils.h>
#include <spa/pod/builder.h>

typedef void (*vpm_fill_fn)(void *ud, int16_t *dst, uint32_t n_samples);

struct vpm_pw {
	struct pw_thread_loop *loop;
	struct pw_context *context;
	struct pw_core *core;
	struct spa_hook core_listener;
	struct pw_stream *stream;
	struct spa_hook stream_listener;
	vpm_fill_fn fill;
	void *ud;
	atomic_int broken;
	atomic_uint node_id;
	atomic_int streaming;
	struct spa_io_position *position; /* set on the loop thread, read in process */
	char error[256];                  /* written/read under the thread-loop lock */
};

static void set_error(struct vpm_pw *p, const char *msg)
{
	snprintf(p->error, sizeof(p->error), "%s", msg ? msg : "unknown error");
}

static void on_process(void *data)
{
	struct vpm_pw *p = data;
	struct pw_buffer *b = pw_stream_dequeue_buffer(p->stream);
	if (b == NULL)
		return;
	struct spa_buffer *buf = b->buffer;
	struct spa_data *d = &buf->datas[0];
	if (d->data == NULL) {
		pw_stream_queue_buffer(p->stream, b);
		return;
	}
	uint32_t stride = sizeof(int16_t);
	uint32_t max = d->maxsize / stride;
	/* Fill what the graph asks for: b->requested (>= 0.3.49), else the current
	 * quantum from the position io, else our node.latency (10 ms). */
	uint32_t n = 0;
	if (b->requested > 0)
		n = (uint32_t)b->requested;
	else if (p->position != NULL)
		n = (uint32_t)p->position->clock.duration;
	if (n == 0)
		n = 1024;
	if (n > max)
		n = max;
	p->fill(p->ud, (int16_t *)d->data, n);
	d->chunk->offset = 0;
	d->chunk->stride = (int32_t)stride;
	d->chunk->size = n * stride;
	pw_stream_queue_buffer(p->stream, b);
}

static void on_state_changed(void *data, enum pw_stream_state old, enum pw_stream_state state, const char *error)
{
	struct vpm_pw *p = data;
	(void)old;
	switch (state) {
	case PW_STREAM_STATE_ERROR:
		set_error(p, error ? error : "stream error");
		atomic_store(&p->broken, 1);
		break;
	case PW_STREAM_STATE_UNCONNECTED:
		set_error(p, "stream disconnected");
		atomic_store(&p->broken, 1);
		break;
	case PW_STREAM_STATE_PAUSED:
	case PW_STREAM_STATE_STREAMING:
		atomic_store(&p->node_id, pw_stream_get_node_id(p->stream));
		atomic_store(&p->streaming, state == PW_STREAM_STATE_STREAMING);
		break;
	default:
		break;
	}
	pw_thread_loop_signal(p->loop, false);
}

static void on_io_changed(void *data, uint32_t id, void *area, uint32_t size)
{
	struct vpm_pw *p = data;
	(void)size;
	if (id == SPA_IO_Position)
		p->position = area;
}

static const struct pw_stream_events stream_events = {
	PW_VERSION_STREAM_EVENTS,
	.state_changed = on_state_changed,
	.io_changed = on_io_changed,
	.process = on_process,
};

static void on_core_error(void *data, uint32_t id, int seq, int res, const char *message)
{
	struct vpm_pw *p = data;
	(void)seq;
	if (id == PW_ID_CORE) {
		set_error(p, res == -EPIPE ? "PipeWire connection lost" : message);
		atomic_store(&p->broken, 1);
		pw_thread_loop_signal(p->loop, false);
	}
}

static const struct pw_core_events core_events = {
	PW_VERSION_CORE_EVENTS,
	.error = on_core_error,
};

void vpm_pw_destroy(struct vpm_pw *p)
{
	if (p == NULL)
		return;
	if (p->loop)
		pw_thread_loop_lock(p->loop);
	if (p->stream) {
		spa_hook_remove(&p->stream_listener);
		pw_stream_destroy(p->stream);
	}
	if (p->core) {
		spa_hook_remove(&p->core_listener);
		pw_core_disconnect(p->core);
	}
	if (p->loop) {
		pw_thread_loop_unlock(p->loop);
		pw_thread_loop_stop(p->loop);
	}
	if (p->context)
		pw_context_destroy(p->context);
	if (p->loop)
		pw_thread_loop_destroy(p->loop);
	free(p);
}

static pthread_once_t init_once = PTHREAD_ONCE_INIT;
static void do_init(void)
{
	pw_init(NULL, NULL);
}

/* Returns NULL on failure (message copied to err). */
struct vpm_pw *vpm_pw_new(const char *node_name, const char *description, uint32_t rate,
			  uint32_t latency_frames, vpm_fill_fn fill, void *ud, char *err, size_t errlen)
{
	pthread_once(&init_once, do_init);
	struct vpm_pw *p = calloc(1, sizeof(*p));
	if (p == NULL) {
		snprintf(err, errlen, "out of memory");
		return NULL;
	}
	p->fill = fill;
	p->ud = ud;
	atomic_store(&p->node_id, SPA_ID_INVALID);

	p->loop = pw_thread_loop_new("vpm-pipewire", NULL);
	if (p->loop == NULL) {
		snprintf(err, errlen, "cannot create PipeWire loop");
		goto fail;
	}
	p->context = pw_context_new(pw_thread_loop_get_loop(p->loop), NULL, 0);
	if (p->context == NULL) {
		snprintf(err, errlen, "cannot create PipeWire context: %s", strerror(errno));
		goto fail;
	}
	if (pw_thread_loop_start(p->loop) < 0) {
		snprintf(err, errlen, "cannot start PipeWire loop");
		goto fail;
	}
	pw_thread_loop_lock(p->loop);
	p->core = pw_context_connect(p->context, NULL, 0);
	if (p->core == NULL) {
		snprintf(err, errlen, "cannot connect to PipeWire: %s", strerror(errno));
		pw_thread_loop_unlock(p->loop);
		goto fail;
	}
	pw_core_add_listener(p->core, &p->core_listener, &core_events, p);

	char latency[64];
	snprintf(latency, sizeof(latency), "%u/%u", latency_frames, rate);
	struct pw_properties *props = pw_properties_new(
		PW_KEY_MEDIA_TYPE, "Audio",
		PW_KEY_MEDIA_CLASS, "Audio/Source" /* "Audio/Source/Virtual" crashes the client-side adapter in PipeWire 1.6.8 */,
		PW_KEY_NODE_NAME, node_name,
		PW_KEY_NODE_NICK, description,
		PW_KEY_NODE_DESCRIPTION, description,
		PW_KEY_NODE_VIRTUAL, "true",
		PW_KEY_PRIORITY_SESSION, "1",
		PW_KEY_NODE_LATENCY, latency,
		PW_KEY_APP_NAME, "Vibe Phone Mic",
		"device.icon-name", "audio-input-microphone",
		NULL);
	p->stream = pw_stream_new(p->core, node_name, props);
	if (p->stream == NULL) {
		snprintf(err, errlen, "cannot create PipeWire stream: %s", strerror(errno));
		pw_thread_loop_unlock(p->loop);
		goto fail;
	}
	pw_stream_add_listener(p->stream, &p->stream_listener, &stream_events, p);

	uint8_t buffer[1024];
	struct spa_pod_builder b = SPA_POD_BUILDER_INIT(buffer, sizeof(buffer));
	struct spa_audio_info_raw info = {
		.format = SPA_AUDIO_FORMAT_S16,
		.rate = rate,
		.channels = 1,
	};
	info.position[0] = SPA_AUDIO_CHANNEL_MONO;
	const struct spa_pod *params[1];
	params[0] = spa_format_audio_raw_build(&b, SPA_PARAM_EnumFormat, &info);

	int res = pw_stream_connect(p->stream, PW_DIRECTION_OUTPUT, PW_ID_ANY,
				    PW_STREAM_FLAG_MAP_BUFFERS | PW_STREAM_FLAG_RT_PROCESS, params, 1);
	if (res < 0) {
		snprintf(err, errlen, "cannot connect PipeWire stream: %s", strerror(-res));
		pw_thread_loop_unlock(p->loop);
		goto fail;
	}
	/* Wait (max ~3 s) until the node exists or something failed. */
	for (int i = 0; i < 3; i++) {
		if (atomic_load(&p->broken) || atomic_load(&p->node_id) != SPA_ID_INVALID)
			break;
		pw_thread_loop_timed_wait(p->loop, 1);
	}
	if (atomic_load(&p->broken)) {
		snprintf(err, errlen, "%s", p->error);
		pw_thread_loop_unlock(p->loop);
		goto fail;
	}
	pw_thread_loop_unlock(p->loop);
	if (atomic_load(&p->node_id) == SPA_ID_INVALID) {
		snprintf(err, errlen, "PipeWire did not create the node in time");
		goto fail;
	}
	return p;
fail:
	vpm_pw_destroy(p);
	return NULL;
}

int vpm_pw_broken(struct vpm_pw *p)
{
	return atomic_load(&p->broken);
}

uint32_t vpm_pw_node_id(struct vpm_pw *p)
{
	return atomic_load(&p->node_id);
}

void vpm_pw_error(struct vpm_pw *p, char *buf, size_t len)
{
	/* error[] is written by callbacks on the loop thread (which hold the lock). */
	pw_thread_loop_lock(p->loop);
	snprintf(buf, len, "%s", p->error);
	pw_thread_loop_unlock(p->loop);
}
