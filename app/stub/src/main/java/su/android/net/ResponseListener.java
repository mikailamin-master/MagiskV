package su.android.net;

public interface ResponseListener<T> {
    void onResponse(T response);
}
