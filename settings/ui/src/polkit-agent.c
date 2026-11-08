/* Authentication is implemented by libpolkit-agent-1/PAM, not by the settings helper. */
#define POLKIT_AGENT_I_KNOW_API_IS_SUBJECT_TO_CHANGE
#include <polkitagent/polkitagent.h>
#include <unistd.h>

typedef void (*UfCallback)(int, const char *, const char *, int, void *);
typedef struct { PolkitAgentListener parent; GTask *task; PolkitAgentSession *session;
    GCancellable *cancel; gulong cancel_id; gchar *message; UfCallback callback; void *data; gpointer registration; } UfAgent;
typedef struct { PolkitAgentListenerClass parent; } UfAgentClass;
G_DEFINE_TYPE(UfAgent, uf_agent, POLKIT_AGENT_TYPE_LISTENER)

static void finish(UfAgent *agent, gboolean canceled) {
    if (!agent->task) return;
    GTask *task=agent->task;agent->task=NULL;
    if (agent->cancel && agent->cancel_id) g_cancellable_disconnect(agent->cancel,agent->cancel_id);
    agent->cancel_id=0;g_clear_object(&agent->cancel);g_clear_object(&agent->session);
    agent->callback(2,"","",0,agent->data);
    if (canceled) g_task_return_new_error(task,G_IO_ERROR,G_IO_ERROR_CANCELLED,"Authentication canceled");
    else g_task_return_boolean(task,TRUE);
    g_object_unref(task);g_clear_pointer(&agent->message,g_free);
}
static void completed(PolkitAgentSession *session, gboolean gained, UfAgent *agent) {(void)session;(void)gained;finish(agent,FALSE);}
static void request(PolkitAgentSession *session,const gchar *prompt,gboolean echo,UfAgent *agent){(void)session;agent->callback(0,prompt,agent->message,echo,agent->data);}
static void information(PolkitAgentSession *session,const gchar *text,UfAgent *agent){(void)session;agent->callback(1,text,agent->message,0,agent->data);}
static void canceled(GCancellable *cancel,UfAgent *agent){(void)cancel;
    /* Do not disconnect a cancellable from inside its own signal handler. */
    agent->cancel_id=0;if(agent->session)polkit_agent_session_cancel(agent->session);finish(agent,TRUE);
}
static void begin(PolkitAgentListener *listener,const gchar *action,const gchar *message,const gchar *icon,
    PolkitDetails *details,const gchar *cookie,GList *identities,GCancellable *cancel,GAsyncReadyCallback callback,gpointer data){
    (void)action;(void)icon;(void)details;UfAgent *agent=(UfAgent *)listener;
    if(agent->task){GTask *task=g_task_new(listener,cancel,callback,data);g_task_return_new_error(task,G_IO_ERROR,G_IO_ERROR_PENDING,"Another authentication is pending");g_object_unref(task);return;}
    if(!identities){GTask *task=g_task_new(listener,cancel,callback,data);g_task_return_new_error(task,G_IO_ERROR,G_IO_ERROR_FAILED,"No administrator identity available");g_object_unref(task);return;}
    PolkitIdentity *identity=identities->data;
    for(GList *i=identities;i;i=i->next){if(POLKIT_IS_UNIX_USER(i->data)&&(guint)polkit_unix_user_get_uid(i->data)==getuid()){identity=i->data;break;}}
    agent->task=g_task_new(listener,cancel,callback,data);agent->message=g_strdup(message);
    agent->session=polkit_agent_session_new(identity,cookie);
    g_signal_connect(agent->session,"request",G_CALLBACK(request),agent);
    g_signal_connect(agent->session,"show-info",G_CALLBACK(information),agent);
    g_signal_connect(agent->session,"show-error",G_CALLBACK(information),agent);
    g_signal_connect(agent->session,"completed",G_CALLBACK(completed),agent);
    if(cancel){agent->cancel=g_object_ref(cancel);agent->cancel_id=g_cancellable_connect(cancel,G_CALLBACK(canceled),agent,NULL);}
    if(agent->session)polkit_agent_session_initiate(agent->session);
}
static gboolean end(PolkitAgentListener *listener,GAsyncResult *result,GError **error){(void)listener;return g_task_propagate_boolean(G_TASK(result),error);}
static void uf_agent_class_init(UfAgentClass *klass){PolkitAgentListenerClass *listener=POLKIT_AGENT_LISTENER_CLASS(klass);listener->initiate_authentication=begin;listener->initiate_authentication_finish=end;}
static void uf_agent_init(UfAgent *agent){(void)agent;}

void *uf_polkit_register(UfCallback callback,void *data){
    GError *error=NULL;PolkitSubject *subject=polkit_unix_session_new_for_process_sync(getpid(),NULL,&error);
    if(!subject){g_clear_error(&error);return NULL;}
    UfAgent *agent=g_object_new(uf_agent_get_type(),NULL);agent->callback=callback;agent->data=data;
    agent->registration=polkit_agent_listener_register(POLKIT_AGENT_LISTENER(agent),POLKIT_AGENT_REGISTER_FLAGS_NONE,subject,NULL,NULL,&error);
    g_object_unref(subject);if(!agent->registration){g_clear_error(&error);g_object_unref(agent);return NULL;}return agent;
}
void uf_polkit_response(void *ptr,const char *response){UfAgent *agent=ptr;if(agent&&agent->session)polkit_agent_session_response(agent->session,response);}
void uf_polkit_cancel(void *ptr){UfAgent *agent=ptr;if(agent&&agent->session)polkit_agent_session_cancel(agent->session);if(agent)finish(agent,TRUE);}
void uf_polkit_unregister(void *ptr){UfAgent *agent=ptr;if(!agent)return;uf_polkit_cancel(ptr);if(agent->registration)polkit_agent_listener_unregister(agent->registration);g_object_unref(agent);}
